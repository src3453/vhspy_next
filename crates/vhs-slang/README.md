# vhs-slang (PoC)

The composite encode → receiver decode path of `vhs-core`, written once in
[Slang](https://shader-slang.org/) and compiled to WGSL (wgpu) and HLSL (Unity,
Unreal). It is a feasibility study for running the simulator inside game
engines without the CPU round trip of `vhs-ffi`.

## What is ported

Only the clean composite path (no degradation stages):

| Kernel | Runs | Threads | Port of |
|---|---|---|---|
| `vhsPrepare` | once per input image | 1 per output pixel, 32×4 tiles cached in groupshared | `LinearFrame::resized` (triangle filter) + `copy_from_linear` + RGB → Y/I/Q or Y/U/V |
| `vhsEncode` | per field | 1 per (line, 64-sample chunk) | `encoder.rs` (Butterworth band limits, quadrature modulation, sync, burst) |
| `vhsSyncAfc` | per field | 1 workgroup, 1 thread per line | `decoder::find_sync_edge` + horizontal AFC (parallel scan), state kept across fields |
| `vhsBurstLock` | per field | 1 workgroup, 1 thread per line | burst detection, PAL ident (parallel vote), subcarrier PLL, ACC, colour killer |
| `vhsSeparateDemod` | per field | 1 per (line, 64-sample chunk) | `fractional_shift` alignment, notch / line comb Y/C separation, synchronous detection |
| `vhsResolve` | per field | 1 per pixel | PAL delay line, Y/C → RGB, writes this field's rows |

How the serial parts are parallelised:

- **Causal IIR filters within a line** are split into chunks. Each thread starts
  its filters with zero state some samples before its chunk. That warm-up
  length is derived from the filter poles (`Biquad.decaySamples`) so that the
  initial-state error decays below `WARMUP_EPS` (1e-6). The first chunk of a
  line starts at sample 0 like the CPU, so it is exact.
- **Horizontal AFC** (`pos ← pos + k·(edge − pos)`) is a chain of affine maps.
  It runs as a Hillis–Steele scan in groupshared memory.
- **PAL ident** is a sum of per-line votes, computed as a tree reduction.
- **The subcarrier PLL** is per-line (parallel) when `burst_pll = 1`.
  For `burst_pll < 1` (the default, 0.2, like a real receiver's APC loop) it
  wraps angles in a nonlinear recurrence, so one thread runs it serially over
  groupshared values. The receiver's crystal offset (`decoder.clocks.chroma.offset_ppm`,
  passed to the shader as `vcxo_offset_hz`) adds a constant per-line phase step to
  the prediction. Clock drift, wow, flutter and jitter, and the horizontal and
  vertical oscillator errors (`decoder.clocks.horizontal` / `vertical`), are not ported.
- **The AFC-aligned signal** is not stored. Kernels interpolate it from the
  composite signal when they read it.
- **Input resampling** runs once per host frame for both fields, not once per
  field. A 1920×1080 input means dozens of texture reads per output pixel, so
  each workgroup loads its input footprint into groupshared once.

### Tube camera (`shaders/camera.slang`, host `GpuCamera`)

A port of `vhs-core/src/camera.rs`. The camera takes an RGBA float input. If the
input is scene-linear, light above white is passed through unclipped, so a
highlight at 8× white leaves a longer comet tail than one at 1×. Otherwise the
input is treated as display values and linearised with a 2.2 gamma. The output
is an `rgba32f` texture at active-video size. Pass it to
`GpuComposite::set_input_texture` to run the composite path on it.

| Kernel | Runs | Port of |
|---|---|---|
| `camOptics` | per input | resize (same triangle filter), exposure, prism crosstalk |
| `camRowSum`, `camMean` | per input | flare mean, summed in the CPU's order |
| `camBlurH`, `camBlurV` | per input | registration shift, flare, Gaussian blur |
| `camTarget` | per field, 1 thread per pixel | charge, photoconductive and capacitive lag, beam limit, ACT, burn-in, hysteresis, preamp |
| `camProcess` | per field, this field's lines | aperture, preamp noise, knee, clip, gamma |

The host uploads the per-tube spread and the sensitivity/dark maps, including
blemishes, from `vhs_core::camera::TubeMaps`, so they match the CPU exactly.
Preamp noise comes from a hash keyed by (seed, field, line, tube, pixel), so it
matches the CPU only statistically. `cargo test -p vhs-slang --release --test camera`
checks the following on an RX 7600 (Vulkan):

- HDR float input at native size and at 1920×1080, NTSC and PAL, with a moving
  6× white highlight over 12 fields: max 0.07 LSB
- the over-white comet tail
- noise mean and standard deviation
- camera → composite against the CPU: max 0.5 LSB

The Unity `.compute` files are generated, but the C# side does not dispatch them yet.

The tape, RF, TBC, 3D comb, playback NR, deinterlacer, capture/DV and
OSD stages are not ported. Signal-degradation parameters (jitter, noise,
attenuation) are ignored. Noise will need a counter-based RNG keyed by
(field, line, sample) instead of `vhs-core`'s sequential streams, so later
stages will match the CPU only statistically.

## Layout

- `shaders/timing.slang`, `shaders/filter.slang`: modules (TV standards, biquad
  and `Cascade<N>` with the stage count as a generic constant).
- `shaders/composite.slang`: the kernels and the `CompositeParams` layout.
- `shaders/camera.slang`: the tube camera kernels (`CameraGpuParams`, `TubeGpu`).
- `shaders/resample.slang`: the input triangle filter, shared by both.
- `generated/`: slangc output, committed so that builds do not need slangc.
  `composite.wgsl` and `camera.wgsl` hold every entry point of their module. `hlsl/<kernel>.hlsl` holds one
  entry point per file, because slangc writes one HLSL file per entry point.
- `src/lib.rs`: wgpu reference host (`GpuComposite`) and the CPU reference
  helpers (`reference::test_pattern`, `reference::cpu_decode`).
- `scripts/build-shaders.ps1`: regenerates the WGSL, the plain HLSL, and the
  Unity `.compute` files in `integrations/unity/Runtime/Resources/VhsNextComposite`.

## Regenerate shaders

```powershell
$env:SLANGC = "C:\path\to\slang\bin\slangc.exe"   # or put slangc on PATH
.\crates\vhs-slang\scripts\build-shaders.ps1
```

This was developed with slangc 2026.22. Parameters go in a one-element
`StructuredBuffer<CompositeParams>` rather than a constant buffer because Unity
cannot reflect a struct inside a cbuffer.

## Verification

```powershell
cargo test -p vhs-slang --release
```

The tests encode and decode a pattern (colour bars, ramps, fine stripes, a
diagonal edge) for four fields on the GPU and on the CPU, then compare the last
frame. They cover:

- NTSC and PAL with notch and line comb separation
- a slow subcarrier PLL (`burst_pll < 1`, the serial path) with non-default AFC
  and chroma bandwidth
- a 1920×1080 scene-linear input downscaled by `vhsPrepare`

The maximum difference in every case is 0.5 LSB (8-bit rounding only),
measured on an RX 7600 with Vulkan and D3D12. The tests are skipped if there is
no GPU adapter.

For engine hosts, `cargo run -p vhs-slang --example dump_reference --release -- <dir> [ntsc|pal] [WxH]`
writes a scene-linear input pattern and the CPU output. A Unity 6000.6.10f1
batch-mode run of the generated `.compute` kernels matched the CPU output to
0.5 LSB on D3D11, D3D12, and Vulkan, for both a native-size input and a
1920×1080 input.

`cargo run -p vhs-slang --example replay --release -- <input.bin> <w> <h> <fields> <output.bin>`
runs a camera frame captured from an engine through this host. Unity's URP
PlayMode check (`integrations/unity/Tests~/UrpGpuComposite`) uses it. Its
decoded raster was bit-identical to the replay on D3D11, D3D12, and Vulkan.

## Performance

`cargo run -p vhs-slang --example bench --release` on an RX 7600 (Vulkan) with
a 1920×1080 RGBA32F input. GPU times are on the GPU timeline after warming the
clocks (idle power states otherwise inflate them by several times). The CPU
column is `vhs-core` with rayon on the same stages at native input size, with
no resize.

| | NTSC | PAL |
|---|---|---|
| GPU per field (all field kernels) | 0.15 ms | 0.18 ms |
| GPU per input image (`vhsPrepare`) | 0.24 ms | 0.26 ms |
| CPU per field | 1.0 ms | 1.6 ms |

Per-kernel times for one NTSC field: Encode 0.040, SyncAfc 0.014, BurstLock
0.020, SeparateDemod 0.048, Resolve 0.015 ms.

The first PoC took about 2 ms per field. The gains came from these steps:

| Change | NTSC ms/field |
|---|---|
| First PoC (1 thread per line, 8 kernels, one submit per field) | 2.0 |
| Chunked IIR with warm-up, fused kernels, batched submits | 0.30 |
| Parallel AFC scan, parallel ident/PLL, 320-thread workgroups, ε = 1e-6 | 0.20 |
| Per-frame `vhsPrepare` (triangle filter, groupshared tiles) | 0.15 + 0.24 per image |

`vhsPrepare` is now close to the memory-bandwidth limit for a 33 MB RGBA32F
image. A half-float camera target, which Unity usually uses for HDR, halves
that cost.

Remaining ideas:

- Run the per-line IIR as a wave-level scan, with each biquad as a 2×2 affine
  recurrence, instead of warm-up chunks.
- Merge `vhsSyncAfc` and `vhsBurstLock` into one workgroup pass. That needs the
  burst gate's aligned samples, which depend on the AFC result.
- Keep the intermediate buffers in half precision where the error budget allows.
