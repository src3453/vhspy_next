using System;
using System.Runtime.InteropServices;
using UnityEngine;
using UnityEngine.Experimental.Rendering;
using UnityEngine.Rendering;

namespace VhsNext
{
    /// <summary>
    /// GPU (compute shader) version of the composite encode → receiver decode path (PoC).
    /// The kernels are generated from crates/vhs-slang/shaders/composite.slang by
    /// crates/vhs-slang/scripts/build-shaders.ps1. Only the clean composite path is ported,
    /// plus the tube camera (<see cref="VhsGpuCamera"/>) when "camera.enabled" is set:
    /// no tape, RF, TBC, noise reduction, 3D comb or capture stages.
    /// </summary>
    public sealed class VhsGpuComposite : IDisposable
    {
        /// <summary>
        /// Index 0 (vhsPrepare) runs once per host frame: it resamples the camera image for both
        /// fields. The rest run once per field, in this order; each reads the previous kernel's buffers.
        /// </summary>
        private static readonly string[] KernelNames =
        {
            "vhsPrepare", "vhsEncode", "vhsSyncAfc", "vhsBurstLock", "vhsSeparateDemod", "vhsResolve"
        };

        // Must match the constants of the same name in composite.slang.
        private const int EncodeChunk = 64;
        private const int DemodChunk = 64;
        private const int PrepareTileW = 32;
        private const int PrepareTileR = 4;

        private const int MaxFieldsPerFrame = 8;

        /// <summary>Same layout as CompositeParams in composite.slang (25 scalars, 100 bytes).</summary>
        [StructLayout(LayoutKind.Sequential)]
        private struct CompositeParams
        {
            public uint standard, parity, frameNo, fieldIndex;
            public uint srcWidth, srcHeight, inputLinear, ycMode;
            public float lumaBwMhz, iBwMhz, qBwMhz, uvBwMhz;
            public float chromaGain, chromaBwMhz, hPll, burstPll;
            public float contrast, brightness, saturation, hueDeg;
            public uint acc, colorKiller, palDelayLine, outputLinear;
            public float vcxoOffsetHz;
        }

        // Subset of the core Params JSON (snake_case like the Rust serde names).
        [Serializable] private sealed class JsonEncoder
        {
            public float luma_bw_mhz = 4.2f, i_bw_mhz = 1.3f, q_bw_mhz = 0.6f, uv_bw_mhz = 1.3f, chroma_gain = 1f;
        }

        [Serializable] private sealed class JsonDecoder
        {
            public string yc_separation = "notch";
            public float chroma_bw_mhz = 1.3f, h_pll = 0.25f, burst_pll = 0.2f;
            public JsonClocks clocks = new JsonClocks();
            public bool acc = true, color_killer = true, pal_delay_line = true;
            public float contrast = 1f, brightness = 0f, saturation = 1f, hue_deg = 0f;
        }

        [Serializable] private sealed class JsonClock
        {
            public float offset_ppm;
        }

        [Serializable] private sealed class JsonClocks
        {
            // Only the chroma VCXO's static offset is ported to the GPU path.
            public JsonClock chroma = new JsonClock { offset_ppm = 14f };
        }

                [Serializable] private sealed class JsonParams
        {
            public string standard = "ntsc";
            public JsonEncoder encoder = new JsonEncoder();
            public JsonDecoder decoder = new JsonDecoder();
            public VhsGpuCamera.Settings camera = new VhsGpuCamera.Settings();
            public ulong seed;
        }

        private static readonly int ParamsId = Shader.PropertyToID("paramBuffer");
        private static readonly int SrcImageId = Shader.PropertyToID("srcImage");
        private static readonly int DstImageId = Shader.PropertyToID("dstImage");
        private static readonly int VideoId = Shader.PropertyToID("video");
        private static readonly int SignalId = Shader.PropertyToID("signal");
        private static readonly int LumaId = Shader.PropertyToID("luma");
        private static readonly int DetectedId = Shader.PropertyToID("detected");
        private static readonly int LinesId = Shader.PropertyToID("lines");
        private static readonly int StateId = Shader.PropertyToID("state");

        private readonly ComputeShader[] _shaders = new ComputeShader[KernelNames.Length];
        private readonly int[] _kernels = new int[KernelNames.Length];
        private readonly GraphicsBuffer[] _paramBuffers = new GraphicsBuffer[MaxFieldsPerFrame];
        private readonly CompositeParams[] _paramData = new CompositeParams[1];
        private readonly GraphicsBuffer _video, _signal, _luma, _detected, _lines, _state;
        private readonly bool _pal;
        private CompositeParams _base;
        private VhsGpuCamera _camera;
        private readonly int _fieldLines;
        private readonly int _activeWidth;
        private readonly int _lineSamples;
        private readonly uint _fieldRateNum;
        private readonly uint _fieldRateDen;
        private readonly uint _fpsNum;
        private readonly uint _fpsDen;
        private ulong _fieldRemainder;
        private ulong _fieldIndex;
        private bool _started;

        /// <summary>Decoded frame (active width × frame lines), scene-linear RGBA float.</summary>
        public RenderTexture Output { get; }

        /// <summary>The tube camera in front of the encoder, or null when "camera.enabled" is off.</summary>
        public VhsGpuCamera Camera => _camera;

        public static bool IsSupported => SystemInfo.supportsComputeShaders;

        public VhsGpuComposite(string paramsJson, uint fpsNumerator, uint fpsDenominator)
        {
            if (!IsSupported)
                throw new InvalidOperationException("Compute shaders are not supported on this device.");
            if (fpsNumerator == 0 || fpsDenominator == 0)
                throw new ArgumentException("Frame rate must be positive.");

            var json = new JsonParams();
            if (!string.IsNullOrWhiteSpace(paramsJson))
                JsonUtility.FromJsonOverwrite(paramsJson, json);
            json.encoder = json.encoder ?? new JsonEncoder();
            json.decoder = json.decoder ?? new JsonDecoder();
            bool pal = string.Equals(json.standard, "pal", StringComparison.OrdinalIgnoreCase);
            _pal = pal;
            if (json.decoder.yc_separation == "comb3d")
                Debug.LogWarning("VHS Next GPU: 3D Y/C separation is not ported yet; using the line comb.");

            int lineSamples = pal ? 1135 : 910;
            _lineSamples = lineSamples;
            _fieldLines = pal ? 288 : 240;
            _activeWidth = pal ? 922 : 754;
            _fieldRateNum = pal ? 50u : 60000u;
            _fieldRateDen = pal ? 1u : 1001u;
            _fpsNum = fpsNumerator;
            _fpsDen = fpsDenominator;

            _base = new CompositeParams
            {
                standard = pal ? 1u : 0u,
                inputLinear = 1,
                ycMode = json.decoder.yc_separation == "notch" ? 0u : 1u,
                lumaBwMhz = json.encoder.luma_bw_mhz,
                iBwMhz = json.encoder.i_bw_mhz,
                qBwMhz = json.encoder.q_bw_mhz,
                uvBwMhz = json.encoder.uv_bw_mhz,
                chromaGain = json.encoder.chroma_gain,
                chromaBwMhz = json.decoder.chroma_bw_mhz,
                hPll = json.decoder.h_pll,
                burstPll = json.decoder.burst_pll,
                contrast = json.decoder.contrast,
                brightness = json.decoder.brightness,
                saturation = json.decoder.saturation,
                hueDeg = json.decoder.hue_deg,
                acc = json.decoder.acc ? 1u : 0u,
                colorKiller = json.decoder.color_killer ? 1u : 0u,
                palDelayLine = json.decoder.pal_delay_line ? 1u : 0u,
                outputLinear = 1,
                vcxoOffsetHz = json.decoder.clocks.chroma.offset_ppm * 1e-6f
                    * (pal ? 4433618.75f : 3579545.45f)
            };

            for (int i = 0; i < KernelNames.Length; ++i)
            {
                ComputeShader shader = Resources.Load<ComputeShader>("VhsNextComposite/" + KernelNames[i]);
                if (shader == null)
                    throw new InvalidOperationException("Missing compute shader VhsNextComposite/" + KernelNames[i]);
                _shaders[i] = shader;
                _kernels[i] = shader.FindKernel(KernelNames[i]);
            }

            int activeCount = _fieldLines * _activeWidth;
            _video = CreateStructured(2 * activeCount, 4 * sizeof(float), "VHS video");
            _signal = CreateStructured(_fieldLines * lineSamples, sizeof(float), "VHS signal");
            _luma = CreateStructured(activeCount, sizeof(float), "VHS luma");
            _detected = CreateStructured(_fieldLines * _activeWidth, 2 * sizeof(float), "VHS detected");
            _lines = CreateStructured(_fieldLines, 8 * sizeof(float), "VHS lines");
            _state = CreateStructured(4, sizeof(float), "VHS state");
            _state.SetData(new float[4]);
            int paramSize = Marshal.SizeOf<CompositeParams>();
            for (int i = 0; i < MaxFieldsPerFrame; ++i)
                _paramBuffers[i] = new GraphicsBuffer(GraphicsBuffer.Target.Structured, 1, paramSize) { name = "VHS params " + i };

            Output = new RenderTexture(_activeWidth, 2 * _fieldLines, 0, GraphicsFormat.R32G32B32A32_SFloat)
            {
                name = "VHS Next GPU output",
                enableRandomWrite = true,
                filterMode = FilterMode.Bilinear,
                wrapMode = TextureWrapMode.Clamp
            };
            if (!Output.Create())
                throw new InvalidOperationException("Could not create the VHS GPU output texture.");

            ApplyCamera(json.camera, json.seed);
        }

        /// <summary>True for PAL (922 × 576), false for NTSC (754 × 480).</summary>
        public bool IsPal => _pal;

        /// <summary>
        /// Turns the tube camera on or off, or changes its parameters, without restarting the composite path.
        /// A running camera keeps its target state (lag, comet tails, burn-in).
        /// </summary>
        public void ApplyCamera(VhsGpuCamera.Settings settings, ulong seed)
        {
            if (settings != null && settings.enabled)
            {
                if (_camera == null)
                    _camera = new VhsGpuCamera(settings, seed, _pal);
                else
                    _camera.Apply(settings, seed);
                // The camera signal is in display values at active-video size
                _base.inputLinear = 0;
            }
            else
            {
                _camera?.Dispose();
                _camera = null;
                _base.inputLinear = 1;
            }
        }

        private static GraphicsBuffer CreateStructured(int count, int stride, string name)
        {
            return new GraphicsBuffer(GraphicsBuffer.Target.Structured, count, stride) { name = name };
        }

        /// <summary>
        /// Records encode/decode dispatches for the fields this host frame advances (same rational
        /// clock as vhs-ffi). <paramref name="source"/> is the scene-linear camera color.
        /// </summary>
        public void Record(CommandBuffer cmd, Texture source)
        {
            _fieldRemainder += (ulong)_fieldRateNum * _fpsDen;
            ulong denominator = (ulong)_fieldRateDen * _fpsNum;
            int fields = (int)(_fieldRemainder / denominator);
            _fieldRemainder %= denominator;
            if (!_started && fields == 0)
                fields = 1;
            _started = true;
            fields = Mathf.Min(fields, MaxFieldsPerFrame);

            for (int f = 0; f < fields; ++f)
            {
                CompositeParams p = _base;
                p.parity = (uint)(_fieldIndex & 1);
                p.frameNo = (uint)(_fieldIndex / 2);
                p.fieldIndex = (uint)_fieldIndex;
                Texture input = source;
                if (_camera != null)
                {
                    _camera.RecordField(cmd, source, _fieldIndex, f == 0);
                    input = _camera.Output;
                }
                p.srcWidth = (uint)input.width;
                p.srcHeight = (uint)input.height;
                _paramData[0] = p;
                // One parameter buffer per field of this frame, so later fields cannot overwrite earlier ones.
                // (A one-element structured buffer rather than a cbuffer: Unity cannot reflect struct cbuffer members.)
                GraphicsBuffer paramBuffer = _paramBuffers[f];
                cmd.SetBufferData(paramBuffer, _paramData);

                // The camera image changes every host frame, so resample it once before its first field.
                // With the tube camera, its output changes every field.
                for (int k = f == 0 || _camera != null ? 0 : 1; k < _shaders.Length; ++k)
                    Dispatch(cmd, k, paramBuffer, input);
                _fieldIndex++;
            }
        }

        private void Dispatch(CommandBuffer cmd, int k, GraphicsBuffer paramBuffer, Texture source)
        {
            ComputeShader cs = _shaders[k];
            int kernel = _kernels[k];
            cmd.SetComputeBufferParam(cs, kernel, ParamsId, paramBuffer);
            cmd.SetComputeTextureParam(cs, kernel, SrcImageId, source);
            cmd.SetComputeTextureParam(cs, kernel, DstImageId, Output);
            cmd.SetComputeBufferParam(cs, kernel, VideoId, _video);
            cmd.SetComputeBufferParam(cs, kernel, SignalId, _signal);
            cmd.SetComputeBufferParam(cs, kernel, LumaId, _luma);
            cmd.SetComputeBufferParam(cs, kernel, DetectedId, _detected);
            cmd.SetComputeBufferParam(cs, kernel, LinesId, _lines);
            cmd.SetComputeBufferParam(cs, kernel, StateId, _state);

            switch (KernelNames[k])
            {
                case "vhsPrepare":
                    cmd.DispatchCompute(cs, kernel, CeilDiv(_activeWidth, PrepareTileW), CeilDiv(2 * _fieldLines, PrepareTileR), 1);
                    break;
                case "vhsEncode":
                    cmd.DispatchCompute(cs, kernel, CeilDiv(_fieldLines * CeilDiv(_lineSamples, EncodeChunk), 64), 1, 1);
                    break;
                case "vhsSyncAfc":
                case "vhsBurstLock":
                    cmd.DispatchCompute(cs, kernel, 1, 1, 1);
                    break;
                case "vhsSeparateDemod":
                    cmd.DispatchCompute(cs, kernel, CeilDiv(_fieldLines * CeilDiv(_activeWidth, DemodChunk), 64), 1, 1);
                    break;
                default: // vhsResolve
                    cmd.DispatchCompute(cs, kernel, CeilDiv(_activeWidth, 8), CeilDiv(_fieldLines, 8), 1);
                    break;
            }
        }

        private static int CeilDiv(int a, int b) => (a + b - 1) / b;

        public void Dispose()
        {
            _video?.Dispose();
            _signal?.Dispose();
            _luma?.Dispose();
            _detected?.Dispose();
            _lines?.Dispose();
            _state?.Dispose();
            _camera?.Dispose();
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
