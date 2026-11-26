using System;
using System.Runtime.InteropServices;
using UnityEngine;
using UnityEngine.Experimental.Rendering;
using UnityEngine.Rendering;

namespace VhsNext
{
    /// <summary>
    /// GPU (compute shader) version of the tube camera (vhs-core camera.rs). The kernels are generated
    /// from crates/vhs-slang/shaders/camera.slang by crates/vhs-slang/scripts/build-shaders.ps1.
    ///
    /// The input is RGBA float light. When it is scene-linear, light above white is passed through
    /// unclipped, which gives comet tails, burn-in and knee from HDR highlights. <see cref="Output"/>
    /// is the camera signal (display values, super white above 1) at active-video size, and is
    /// meant to be the source of <see cref="VhsGpuComposite"/>.
    ///
    /// The per-tube spread and the sensitivity / dark-current maps (with blemishes) are made here on
    /// the CPU with the same 64-bit RNG as vhs-core, so they match the native camera exactly.
    /// Preamp noise uses a GPU hash, so it matches the native camera only statistically.
    /// </summary>
    public sealed class VhsGpuCamera : IDisposable
    {
        /// <summary>Kernels run when the source image changes.</summary>
        private static readonly string[] OpticsKernels = { "camOptics", "camRowSum", "camMean", "camBlurH", "camBlurV" };
        /// <summary>Kernels run once per field.</summary>
        private static readonly string[] FieldKernels = { "camTarget", "camProcess" };

        private const float AmpBias = 0.35f;
        private const int StateStride = 5 * 4 * sizeof(float);
        private const int MaxFieldsPerFrame = 8;

        /// <summary>Same layout as CameraGpuParams in camera.slang (36 scalars, 144 bytes).</summary>
        [StructLayout(LayoutKind.Sequential)]
        private struct CameraGpuParams
        {
            public uint width, height, srcWidth, srcHeight;
            public uint inputLinear, parity, prime, fieldIndex;
            public uint seedLo, seedHi, act, pad0;
            public float gain, crosstalk, flare, sigma;
            public float kDecay, kRise, actLevel, bias;
            public float burnRecovery, hysteresis, hRelax, ampCp;
            public float ampCn, ampF0, ampScale, procGain;
            public float noise, aperture, knee, kneeSlope;
            public float whiteClip, cameraGamma, pad1, pad2;
        }

        /// <summary>Same layout as TubeGpu in camera.slang (12 floats, 48 bytes).</summary>
        [StructLayout(LayoutKind.Sequential)]
        private struct TubeGpu
        {
            public float gamma, dx, dy, scale;
            public float photoLag, capLag, beam, qsat;
            public float black, burnRate, pad0, pad1;
        }

        /// <summary>vhs-core camera::Tube (the per-tube spread).</summary>
        public struct Tube
        {
            public float Gamma, Sens, CapLag, PhotoLag, Dark, Beam, Burn, Black, Dx, Dy, Scale, ShadeX, ShadeY;
        }

        /// <summary>
        /// The camera section of the core Params JSON (snake_case like the Rust serde names, same defaults).
        /// Serializable, so it can be a [SerializeField] of a component; the attributes drive the Inspector.
        /// </summary>
        [Serializable]
        public sealed class Settings
        {
            [Tooltip("Simulate the tube camera in front of the encoder.")]
            public bool enabled;

            // --- Optics ---
            [Tooltip("Exposure (iris) in EV.")]
            [Range(-4f, 6f)] public float exposure_ev = 0f;
            [Tooltip("Dichroic prism crosstalk: light leaking into the neighbouring colour.")]
            [Range(0f, 0.3f)] public float crosstalk = 0.03f;
            [Tooltip("Lens and prism flare: share of the average picture light added everywhere (twice for the red tube).")]
            [Range(0f, 0.2f)] public float flare = 0.02f;
            [Tooltip("Registration error between the R, G and B tube images, in pixels.")]
            [Range(0f, 5f)] public float registration_px = 0.4f;
            [Tooltip("Sensitivity fall-off towards the edges (shading).")]
            [Range(0f, 0.5f)] public float shading = 0.08f;
            [Tooltip("Blur from the lens, target and beam spot, in pixels.")]
            [Range(0f, 4f)] public float resolution_px = 0.7f;

            // --- Photoconductive target ---
            [Tooltip("Target gamma (light to signal current). Vidicon about 0.65, Plumbicon 0.95, Saticon 1.0.")]
            [Range(0.2f, 2f)] public float target_gamma = 0.95f;
            [Tooltip("Dark current (relative to white).")]
            [Range(0f, 0.1f)] public float dark_current = 0.005f;
            [Tooltip("Photoconductive lag (traps): a slow response that trails moving objects.")]
            [Range(0f, 0.95f)] public float photo_lag = 0.05f;
            [Tooltip("Decay time constant of the photoconductive lag, in fields (the rise takes half as long).")]
            [Range(0.1f, 60f)] public float photo_lag_fields = 6f;
            [Tooltip("Capacitive (beam) lag: the beam leaves charge behind, more so in dark areas.")]
            [Range(0f, 1f)] public float capacitive_lag = 0.08f;
            [Tooltip("Bias light (relative to white). Reduces lag.")]
            [Range(0f, 0.2f)] public float bias_light = 0f;
            [Tooltip("Beam current limit (relative to white). Charge above it is left behind as a comet tail.")]
            [Range(0.05f, 10f)] public float beam_current = 2f;
            [Tooltip("Charge the target can hold (relative to white).")]
            [Range(1f, 50f)] public float target_saturation = 12f;
            [Tooltip("Anti-comet-tail: discharge excess charge during retrace.")]
            public bool act;
            [Tooltip("Charge level at which ACT starts discharging (relative to white).")]
            [Range(0.1f, 10f)] public float act_level = 1.5f;
            [Tooltip("Target hysteresis width (relative to white): light-to-dark and dark-to-light leave different images.")]
            [Range(0f, 0.2f)] public float hysteresis = 0f;
            [Tooltip("Time constant for the hysteresis to relax, in fields.")]
            [Range(0.1f, 300f)] public float hysteresis_fields = 30f;
            [Tooltip("Burn-in rate per second where the light is above white.")]
            [Range(0f, 5f)] public float burn_rate = 0.02f;
            [Tooltip("Burn-in recovery rate per second.")]
            [Range(0f, 5f)] public float burn_recovery = 0.05f;
            [Tooltip("Number of blemishes (dark stains and white spots) on the targets.")]
            [Range(0, 200)] public uint blemishes;

            // --- Tube preamp ---
            [Tooltip("Preamp non-linearity (0 = linear).")]
            [Range(0f, 3f)] public float amp_drive = 0.3f;
            [Tooltip("Preamp asymmetry: positive compresses whites more, negative compresses blacks more.")]
            [Range(-1f, 1f)] public float amp_asymmetry = 0.4f;
            [Tooltip("Preamp noise (rms, %). Rises with frequency (triangular noise).")]
            [Range(0f, 10f)] public float noise_pct = 0.8f;

            // --- Tube spread and ageing ---
            [Tooltip("Spread between the R, G and B tubes (gamma, lag, sensitivity, black level and so on).")]
            [Range(0f, 1f)] public float mismatch = 0.2f;
            [Tooltip("Tube ageing: lower sensitivity and beam current, more lag, burn-in, blemishes and noise.")]
            [Range(0f, 1f)] public float aging = 0f;

            // --- Processing ---
            [Tooltip("Aperture (edge) correction.")]
            [Range(0f, 3f)] public float aperture = 0.5f;
            [Tooltip("Knee start level (relative to white).")]
            [Range(0.5f, 2f)] public float knee = 0.9f;
            [Tooltip("Slope above the knee.")]
            [Range(0f, 1f)] public float knee_slope = 0.3f;
            [Tooltip("White clip level (super white above 1 stays in the signal).")]
            [Range(0.8f, 2f)] public float white_clip = 1.08f;
            [Tooltip("Camera gamma correction.")]
            [Range(0.3f, 1f)] public float camera_gamma = 0.45f;

            public Settings Clone() => (Settings)MemberwiseClone();
        }

        private static readonly int ParamsId = Shader.PropertyToID("camParams");
        private static readonly int TubesId = Shader.PropertyToID("camTubes");
        private static readonly int SourceId = Shader.PropertyToID("camSource");
        private static readonly int OutputId = Shader.PropertyToID("camOutput");
        private static readonly int MapsId = Shader.PropertyToID("camMaps");
        private static readonly int LinId = Shader.PropertyToID("camLin");
        private static readonly int TmpId = Shader.PropertyToID("camTmp");
        private static readonly int FluxId = Shader.PropertyToID("camFlux");
        private static readonly int StateId = Shader.PropertyToID("camState");
        private static readonly int RowsId = Shader.PropertyToID("camRows");

        private readonly ComputeShader[] _optics = new ComputeShader[OpticsKernels.Length];
        private readonly int[] _opticsKernels = new int[OpticsKernels.Length];
        private readonly ComputeShader[] _field = new ComputeShader[FieldKernels.Length];
        private readonly int[] _fieldKernels = new int[FieldKernels.Length];
        private readonly GraphicsBuffer[] _paramBuffers = new GraphicsBuffer[MaxFieldsPerFrame];
        private readonly CameraGpuParams[] _paramData = new CameraGpuParams[1];
        private readonly GraphicsBuffer _tubes, _maps, _lin, _tmp, _flux, _state, _rows;
        private readonly int _width, _height;
        private readonly float _fieldRate;
        private CameraGpuParams _base;
        private Settings _settings;
        private ulong _seed;
        private bool _opticsDirty = true;
        private int _slot;
        private ulong? _lastField;

        /// <summary>Camera signal (active width × frame lines), RGBA float display values.</summary>
        public RenderTexture Output { get; }

        /// <summary>The per-tube spread, the same values as vhs-core camera::TubeMaps.</summary>
        public Tube[] Tubes { get; private set; }

        /// <param name="settings">Camera parameters (the "camera" object of the core Params JSON).</param>
        /// <param name="seed">The Params seed.</param>
        /// <param name="pal">PAL (922 × 576) instead of NTSC (754 × 480).</param>
        public VhsGpuCamera(Settings settings, ulong seed, bool pal)
        {
            if (!SystemInfo.supportsComputeShaders)
                throw new InvalidOperationException("Compute shaders are not supported on this device.");
            settings = settings ?? new Settings();
            _width = pal ? 922 : 754;
            _height = pal ? 576 : 480;
            _fieldRate = pal ? 50f : 60000f / 1001f;

            Load(OpticsKernels, _optics, _opticsKernels);
            Load(FieldKernels, _field, _fieldKernels);

            int n = _width * _height;
            _maps = Structured(2 * n, 16, "VHS camera maps");
            _tubes = Structured(3, Marshal.SizeOf<TubeGpu>(), "VHS camera tubes");

            _lin = Structured(n, 16, "VHS camera lin");
            _tmp = Structured(n, 16, "VHS camera tmp");
            _flux = Structured(n, 16, "VHS camera flux");
            _state = Structured(n, StateStride, "VHS camera state");
            // The target state of lines not yet read must start at zero, as on the CPU
            _state.SetData(new float[n * StateStride / sizeof(float)]);
            _rows = Structured(_height + 1, 16, "VHS camera rows");
            for (int i = 0; i < MaxFieldsPerFrame; ++i)
                _paramBuffers[i] = Structured(1, Marshal.SizeOf<CameraGpuParams>(), "VHS camera params " + i);

            Output = new RenderTexture(_width, _height, 0, GraphicsFormat.R32G32B32A32_SFloat)
            {
                name = "VHS Next camera output",
                enableRandomWrite = true,
                filterMode = FilterMode.Point,
                wrapMode = TextureWrapMode.Clamp
            };
            if (!Output.Create())
                throw new InvalidOperationException("Could not create the VHS camera output texture.");
            Apply(settings, seed);
        }

        /// <summary>
        /// Changes the parameters without resetting the target state (lag, comet tails and burn-in carry on).
        /// The tube spread and blemish maps are rebuilt only when the seed or a parameter they depend on changes.
        /// </summary>
        public void Apply(Settings settings, ulong seed)
        {
            settings = (settings ?? new Settings()).Clone();
            if (_settings == null || seed != _seed || MapsDiffer(_settings, settings))
            {
                int n = _width * _height;
                Tubes = GenerateTubes(settings, seed);
                BuildMaps(settings, seed, Tubes, _width, _height, out float[][] sens, out float[][] dark);
                var packed = new Vector4[2 * n];
                for (int i = 0; i < n; ++i)
                {
                    packed[2 * i] = new Vector4(sens[0][i], sens[1][i], sens[2][i], 0f);
                    packed[2 * i + 1] = new Vector4(dark[0][i], dark[1][i], dark[2][i], 0f);
                }
                _maps.SetData(packed);
            }
            var tubes = new TubeGpu[3];
            for (int c = 0; c < 3; ++c)
                tubes[c] = MakeTube(settings, Tubes[c], _fieldRate);
            _tubes.SetData(tubes);

            _base = MakeParams(settings, seed, _fieldRate);
            _base.width = (uint)_width;
            _base.height = (uint)_height;
            _settings = settings;
            _seed = seed;
            _opticsDirty = true;
        }

        /// <summary>The parameters the maps depend on (vhs-core camera::MapKey).</summary>
        private static bool MapsDiffer(Settings a, Settings b)
        {
            return a.shading != b.shading || a.aging != b.aging || a.mismatch != b.mismatch || a.dark_current != b.dark_current
                || a.target_gamma != b.target_gamma || a.registration_px != b.registration_px || a.blemishes != b.blemishes;
        }

        private static void Load(string[] names, ComputeShader[] shaders, int[] kernels)
        {
            for (int i = 0; i < names.Length; ++i)
            {
                ComputeShader shader = Resources.Load<ComputeShader>("VhsNextComposite/" + names[i]);
                if (shader == null)
                    throw new InvalidOperationException("Missing compute shader VhsNextComposite/" + names[i]);
                shaders[i] = shader;
                kernels[i] = shader.FindKernel(names[i]);
            }
        }

        private static GraphicsBuffer Structured(int count, int stride, string name)
        {
            return new GraphicsBuffer(GraphicsBuffer.Target.Structured, count, stride) { name = name };
        }

        /// <summary>
        /// Records one field (vhs-core CameraSim::process). Only this field's lines of <see cref="Output"/>
        /// are written. Set <paramref name="sourceChanged"/> when <paramref name="source"/> holds a new
        /// image (the optics run only then). Call at most 8 times per command-buffer execution.
        /// </summary>
        /// <param name="sourceLinear">The source is scene-linear light (white = 1, no upper limit);
        /// otherwise display values in 0..1, linearised with a 2.2 gamma.</param>
        public void RecordField(CommandBuffer cmd, Texture source, ulong fieldIndex, bool sourceChanged, bool sourceLinear = true)
        {
            // A gap in the field sequence (first field, or resumed after being disabled) restarts from steady state
            bool prime = !_lastField.HasValue || _lastField.Value + 1 != fieldIndex;
            _lastField = fieldIndex;

            CameraGpuParams p = _base;
            p.srcWidth = (uint)source.width;
            p.srcHeight = (uint)source.height;
            p.inputLinear = sourceLinear ? 1u : 0u;
            p.parity = (uint)(fieldIndex & 1);
            p.prime = prime ? 1u : 0u;
            p.fieldIndex = (uint)fieldIndex;
            _paramData[0] = p;
            // One parameter buffer per field recorded into the same command buffer
            GraphicsBuffer paramBuffer = _paramBuffers[_slot];
            _slot = (_slot + 1) % MaxFieldsPerFrame;
            cmd.SetBufferData(paramBuffer, _paramData);

            if (sourceChanged || prime || _opticsDirty)
            {
                _opticsDirty = false;
                for (int k = 0; k < _optics.Length; ++k)
                    Dispatch(cmd, _optics[k], _opticsKernels[k], OpticsKernels[k], paramBuffer, source);
            }
            for (int k = 0; k < _field.Length; ++k)
                Dispatch(cmd, _field[k], _fieldKernels[k], FieldKernels[k], paramBuffer, source);
        }

        private void Dispatch(CommandBuffer cmd, ComputeShader cs, int kernel, string name, GraphicsBuffer paramBuffer, Texture source)
        {
            cmd.SetComputeBufferParam(cs, kernel, ParamsId, paramBuffer);
            cmd.SetComputeBufferParam(cs, kernel, TubesId, _tubes);
            cmd.SetComputeTextureParam(cs, kernel, SourceId, source);
            cmd.SetComputeTextureParam(cs, kernel, OutputId, Output);
            cmd.SetComputeBufferParam(cs, kernel, MapsId, _maps);
            cmd.SetComputeBufferParam(cs, kernel, LinId, _lin);
            cmd.SetComputeBufferParam(cs, kernel, TmpId, _tmp);
            cmd.SetComputeBufferParam(cs, kernel, FluxId, _flux);
            cmd.SetComputeBufferParam(cs, kernel, StateId, _state);
            cmd.SetComputeBufferParam(cs, kernel, RowsId, _rows);
            switch (name)
            {
                case "camRowSum":
                    cmd.DispatchCompute(cs, kernel, CeilDiv(_height, 64), 1, 1);
                    break;
                case "camMean":
                    cmd.DispatchCompute(cs, kernel, 1, 1, 1);
                    break;
                case "camProcess":
                    cmd.DispatchCompute(cs, kernel, CeilDiv(_width, 8), CeilDiv(_height / 2, 8), 1);
                    break;
                default:
                    cmd.DispatchCompute(cs, kernel, CeilDiv(_width, 8), CeilDiv(_height, 8), 1);
                    break;
            }
        }

        private static int CeilDiv(int a, int b) => (a + b - 1) / b;

        // ---------------------------------------------------------------------------------------
        // Same formulas as vhs-core camera.rs (and vhs-slang src/camera.rs)
        // ---------------------------------------------------------------------------------------

        private static float Clamp01(float v) => Math.Min(Math.Max(v, 0f), 1f);

        private static CameraGpuParams MakeParams(Settings p, ulong seed, float fieldRate)
        {
            float a = Clamp01(p.aging);
            float tau = Math.Max(p.photo_lag_fields, 0.1f);
            float drive = Math.Max(p.amp_drive, 0f);
            float asym = Math.Min(Math.Max(p.amp_asymmetry, -1f), 1f);
            float cp = drive * (1f + asym), cn = drive * (1f - asym);
            float Raw(float s)
            {
                float u = s - AmpBias;
                float c = u > 0f ? cp : cn;
                return c < 1e-4f ? u : MathF.Tanh(c * u) / c;
            }
            float f0 = Raw(0f), f1 = Raw(1f);
            return new CameraGpuParams
            {
                seedLo = (uint)seed,
                seedHi = (uint)(seed >> 32),
                act = p.act ? 1u : 0u,
                gain = MathF.Pow(2f, p.exposure_ev),
                crosstalk = Math.Min(Math.Max(p.crosstalk, 0f), 0.33f),
                flare = Math.Max(p.flare, 0f),
                sigma = Math.Max(p.resolution_px, 0f) + 0.8f * a,
                kDecay = 1f - MathF.Exp(-1f / tau),
                kRise = 1f - MathF.Exp(-2f / tau),
                actLevel = Math.Max(p.act_level, 0.1f) * 2f,
                bias = Math.Max(p.bias_light, 0f),
                burnRecovery = Math.Max(p.burn_recovery, 0f) / fieldRate,
                hysteresis = Math.Max(p.hysteresis, 0f),
                hRelax = 1f - MathF.Exp(-1f / Math.Max(p.hysteresis_fields, 0.1f)),
                ampCp = cp,
                ampCn = cn,
                ampF0 = f0,
                ampScale = Math.Abs(f1 - f0) > 1e-6f ? 1f / (f1 - f0) : 1f,
                procGain = 1f / (1f - 0.4f * a),
                noise = Math.Max(p.noise_pct, 0f) * 0.01f * (1f + 2f * a),
                aperture = p.aperture,
                knee = p.knee,
                kneeSlope = Math.Max(p.knee_slope, 0f),
                whiteClip = p.white_clip,
                cameraGamma = Math.Max(p.camera_gamma, 0.05f)
            };
        }

        private static TubeGpu MakeTube(Settings p, Tube t, float fieldRate)
        {
            float a = Clamp01(p.aging);
            float beam = Math.Max(Math.Max(p.beam_current, 0.05f) * t.Beam * 2f, 0.05f);
            return new TubeGpu
            {
                gamma = t.Gamma,
                dx = t.Dx,
                dy = t.Dy,
                scale = t.Scale,
                photoLag = Math.Min(Math.Max(p.photo_lag, 0f) * t.PhotoLag + 0.3f * a, 0.95f),
                capLag = Math.Max(p.capacitive_lag, 0f) * t.CapLag,
                beam = beam,
                qsat = Math.Max(Math.Max(p.target_saturation, p.beam_current) * 2f, beam),
                black = Math.Max(p.dark_current, 0f) * (1f + 5f * a) + Math.Max(p.bias_light, 0f) - t.Black,
                burnRate = Math.Max(p.burn_rate, 0f) * t.Burn / fieldRate
            };
        }

        /// <summary>vhs-core rng::Rng (SplitMix64).</summary>
        private struct Rng
        {
            private ulong _state;

            private static ulong Mix(ulong z)
            {
                z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9UL;
                z = (z ^ (z >> 27)) * 0x94D049BB133111EBUL;
                return z ^ (z >> 31);
            }

            public Rng(ulong seed) { _state = Mix(seed ^ 0x9E3779B97F4A7C15UL); }

            private ulong Next()
            {
                unchecked { _state += 0x9E3779B97F4A7C15UL; }
                return Mix(_state);
            }

            public float Uniform() => (Next() >> 40) * (1f / (1 << 24));

            public float Normal()
            {
                ulong v = Next();
                float a = (float)(v & 0xFFFF) + (float)((v >> 16) & 0xFFFF) + (float)((v >> 32) & 0xFFFF) + (float)(v >> 48);
                return (a - 2f * 65535f) * (1f / (65536f * 0.5773503f));
            }

            public float Exponential(float mean) => -mean * MathF.Log(Math.Max(1f - Uniform(), 1e-7f));
        }

        /// <summary>vhs-core camera::Tube::generate.</summary>
        public static Tube[] GenerateTubes(Settings p, ulong seed)
        {
            float a = Clamp01(p.aging);
            float m = Math.Max(p.mismatch, 0f) + 0.5f * a;
            var rng = new Rng(seed ^ 0xCA3E7A115EED0001UL);
            var tubes = new Tube[3];
            for (int c = 0; c < 3; ++c)
            {
                var t = new Tube();
                t.Gamma = Math.Min(Math.Max(p.target_gamma * (1f + 0.06f * m * rng.Normal()), 0.2f), 2f);
                t.Sens = Math.Max(1f + 0.08f * m * rng.Normal(), 0.3f) * (1f - 0.4f * a);
                t.CapLag = MathF.Exp(0.4f * m * rng.Normal()) * (1f + 3f * a);
                t.PhotoLag = MathF.Exp(0.4f * m * rng.Normal());
                t.Dark = Math.Max(1f + 0.5f * m * rng.Normal(), 0f) * (1f + 5f * a);
                t.Beam = Math.Max(1f + 0.15f * m * rng.Normal(), 0.2f) * (1f - 0.6f * a);
                t.Burn = MathF.Exp(0.5f * m * rng.Normal()) * (1f + 4f * a);
                t.Black = 0.01f * m * rng.Normal();
                t.Scale = 1f + 0.002f * p.registration_px * rng.Normal();
                t.ShadeX = 0.5f * m * rng.Normal();
                t.ShadeY = 0.5f * m * rng.Normal();
                // The R and B tubes are displaced relative to the G tube
                float angle = rng.Uniform() * MathF.PI * 2f;
                float r = p.registration_px * (0.5f + 0.5f * rng.Uniform()) * (c == 1 ? 0.3f : 1f);
                t.Dx = r * MathF.Cos(angle);
                t.Dy = r * MathF.Sin(angle);
                tubes[c] = t;
            }
            return tubes;
        }

        /// <summary>Rust f32::signum (−0.0 is negative, +0.0 is positive).</summary>
        private static float Signum(float v) => float.IsNegative(v) ? -1f : 1f;

        /// <summary>vhs-core CameraSim::build_maps: sensitivity (shading, blemishes) and dark current (tilt, white spots).</summary>
        public static void BuildMaps(Settings p, ulong seed, Tube[] tubes, int w, int h, out float[][] sens, out float[][] dark)
        {
            float a = Clamp01(p.aging);
            float shading = Math.Max(p.shading, 0f) + 0.2f * a;
            float darkLevel = Math.Max(p.dark_current, 0f);
            sens = new float[3][];
            dark = new float[3][];
            for (int c = 0; c < 3; ++c)
            {
                Tube t = tubes[c];
                var s = sens[c] = new float[w * h];
                var d = dark[c] = new float[w * h];
                for (int y = 0; y < h; ++y)
                {
                    float ny = (y + 0.5f) / h * 2f - 1f;
                    for (int x = 0; x < w; ++x)
                    {
                        float nx = (x + 0.5f) / w * 2f - 1f;
                        float r2 = 0.5f * (nx * nx + ny * ny);
                        float tilt = 1f + shading * (t.ShadeX * nx + t.ShadeY * ny);
                        s[y * w + x] = Math.Max(t.Sens * (1f - shading * r2) * tilt, 0f);
                        d[y * w + x] = darkLevel * t.Dark * Math.Max(1f + 0.6f * nx * Signum(t.ShadeX) + 0.4f * r2, 0f);
                    }
                }
            }

            int count = (int)p.blemishes + (int)(40f * a);
            var rng = new Rng(seed ^ 0xB1E315E500000002UL);
            for (int k = 0; k < count; ++k)
            {
                int c = (int)(rng.Uniform() * 3f) % 3;
                float cx = rng.Uniform() * w;
                float cy = rng.Uniform() * h;
                float radius = Math.Min(0.8f + rng.Exponential(2f), 12f);
                bool white = rng.Uniform() < 0.2f;
                float strength = 0.2f + 0.7f * rng.Uniform();
                int reach = (int)MathF.Ceiling(radius * 2.5f);
                for (int yy = Math.Max((int)cy - reach, 0); yy < Math.Min((int)cy + reach, h); ++yy)
                {
                    for (int xx = Math.Max((int)cx - reach, 0); xx < Math.Min((int)cx + reach, w); ++xx)
                    {
                        float dx = (xx + 0.5f - cx) / radius;
                        float dy = (yy + 0.5f - cy) / radius;
                        float g = MathF.Exp(-(dx * dx + dy * dy));
                        int i = yy * w + xx;
                        if (white)
                            dark[c][i] += 0.25f * strength * g;
                        else
                            sens[c][i] *= 1f - strength * g;
                    }
                }
            }
        }

        public void Dispose()
        {
            _tubes?.Dispose();
            _maps?.Dispose();
            _lin?.Dispose();
            _tmp?.Dispose();
            _flux?.Dispose();
            _state?.Dispose();
            _rows?.Dispose();
            foreach (GraphicsBuffer buffer in _paramBuffers)
                buffer?.Dispose();
            if (Output != null)
            {
                Output.Release();
                if (Application.isPlaying)
                    UnityEngine.Object.Destroy(Output);
                else
                    UnityEngine.Object.DestroyImmediate(Output);
            }
        }
    }
}
