# VHS Next for Unity

This package adds an asynchronous bridge from a Unity camera render target to
the existing `vhs-ffi` processor. The included adapters target Unity 6 with URP
17+ or HDRP 17+. URP uses a Render Graph pass; HDRP uses a Custom Pass Volume.

The Rust simulator runs on a worker thread. Unity reads back scene-linear
`RGBAFloat` frames asynchronously, sends them to one persistent processor per
camera, then displays the newest completed output on later frames. Startup
displays the camera image until the first processed frame is ready. The image is
center-cropped to the configured aspect ratio and centered on a black canvas;
the default 4:3 ratio produces black side bars on a 16:9 output. If
processing falls behind, capture pauses at the configured queue limit and
resumes when the worker catches up. This first version supports ordinary 2D
camera targets; XR, back-buffer targets, and deterministic offline sequence
export are not covered by these adapters.

## Install

1. The package includes a Windows x86_64 native plugin. If rebuilding it or
   targeting another platform, build the library from the repository root:

   ```powershell
   cargo build -p vhs-ffi --release
   ```

   Replace `Runtime/Plugins/x86_64/vhs_ffi.dll` with the rebuilt Windows DLL.
2. In Unity Package Manager, choose **Add package from disk** and select this
   folder's `package.json`.
3. In the package's **Samples** section, import the URP or HDRP adapter you use.
4. Add `VhsUnityEffect` to the camera to process. The default clock is 60/1
   frames per second. Set it to match the source camera rate.
   `Output Aspect Width` and `Output Aspect Height` default to 4 and 3.

## Parameters

**Source** on `VhsUnityEffect` selects where the simulation parameters come
from:

- **Inspector** (default): the component's own serialized fields, which are
  **Standard**, **Seed** and **Tube Camera**. Undo, prefab overrides and
  multi-object editing work as for any component. The tube camera settings are
  grouped into Optics, Photoconductive Target, Beam and Burn-in, Tube Preamp,
  Tube Spread and Ageing, and Processing. Every field has a tooltip. The
  **Tube Type** buttons set the target gamma, lag and dark current to rough
  Vidicon, Plumbicon or Saticon values. The tape, RF and receiver stages keep
  the core defaults.
- **Json Asset**: a VHS Next Params JSON `TextAsset`, the same format as the
  desktop app's presets. An empty field uses the core defaults.

**Copy Params JSON** copies the Params JSON for the current settings to the
clipboard, as a starting point for a JSON asset.

In Play mode, Inspector edits take effect while the scene runs. With **Gpu
Composite**, a tube camera edit is applied in place, so lag, comet tails and
burn-in carry on. Any other edit, and every edit with **Native Cpu**, restarts
the simulator.

The included native binary instructions are Windows x86_64. Build the Rust
library for the target platform and place it in Unity's platform-specific
`Assets/Plugins` folder for other targets.

## URP

Add `VhsNextRendererFeature` to the active URP Renderer. Its default injection
point is before post-processing so the input matches the native processor's
scene-linear frame contract; later Unity post-processing will still affect the
VHS output. Attach `VhsUnityEffect` to each camera that should be processed.

## HDRP

Create a global Custom Pass Volume, add `VhsNextCustomPass`, and select
**Before Post Process** so the input matches the native processor's scene-linear
frame contract. Later HDRP post-processing will still affect the VHS output.
Attach `VhsUnityEffect` to the camera.

## GPU composite backend (PoC)

Set **Backend** on `VhsUnityEffect` to **Gpu Composite** to run compute
shaders generated from `crates/vhs-slang` (Slang) instead of the native DLL.
The kernels run in the same frame, with no `AsyncGPUReadback` or worker thread.
Only the tube camera (when **Tube Camera** / `camera.enabled` is on) and the
clean composite encode → receiver decode path are ported. The tape, RF, TBC, 3D
comb, playback NR, deinterlacer, capture and OSD stages are skipped, and
`comb3d` falls back to the line comb. The params JSON reads `standard`, `seed`,
`camera.*`, `encoder.*` and the matching `decoder.*` fields.

The tube camera takes the scene-linear camera colour, so HDR highlights above
white give longer comet tails and stronger burn-in than clipped ones. Its tube
spread and blemish maps match the native camera exactly. Its preamp noise
matches only statistically. See `Tests~/GpuCamera` for the comparison with the
CPU camera.

The kernels write the 754×480 (PAL 922×576) raster into a float render texture.
That raster stands for the whole camera frame, as in the native path, and the
4:3 aperture is applied when presenting. The camera image is resampled with the
same triangle filter as the native path, once per frame for both fields.

On an RX 7600, the wgpu reference host in `crates/vhs-slang` measures about
0.15 ms per NTSC field plus 0.24 ms per 1920×1080 RGBA32F camera image. Inside
URP at 1280×720 and 60 fps, the `VHS Next` render-graph pass takes about
0.47 ms (D3D11) or 0.50 ms (Vulkan) of GPU time per frame, and 0.03–0.05 ms of
CPU time.

The generated `.compute` files are in `Runtime/Resources/VhsNextComposite`.
Regenerate them with `crates/vhs-slang/scripts/build-shaders.ps1`; do not edit
them by hand. A batch-mode Unity 6000.6.10f1 run of these kernels matched the
CPU simulator to 0.5 LSB on D3D11, D3D12, and Vulkan, with native-size and
1920×1080 inputs.

The URP adapter in GPU mode was checked in a PlayMode scene on D3D11, D3D12,
and Vulkan, using the harness in `Tests~/UrpGpuComposite`. The decoded raster
was bit-identical to the wgpu reference fed with the same camera frame. The
side bars were black, and the presented frame had the camera's orientation.
The HDRP adapter in GPU mode has been compiled but not run.

## Notes

- The processor keeps state across frames. Use one effect component per camera
  whose output should have independent tape state.
- `AsyncGPUReadback` must be supported by the graphics device. Without it, the
  adapter displays the camera image through the configured aspect aperture and
  logs a warning; VHS processing is skipped.
- At the queue limit, incoming frames are skipped until the worker catches up;
  tape state advances for processed frames only. Lower the camera render
  resolution or queue limit if memory use is too high.
- The current decoder resolves internally to 8-bit video, so the float render
  target preserves the host transfer path and HDR input handling but does not
  add decoded VHS precision.
- For offline frame sequences, create one `VhsNativeProcessor` for the whole
  sequence and call `Process(inputRgba, outputRgba, width, height, alphaMode)`
  once per frame in increasing order. This direct API lets an editor export
  tool wait for each render-target readback, process it deterministically, and
  save the result without the realtime adapter's frame queue. Reuse the same
  buffers and processor for the sequence:

  ```csharp
  using (var processor = new VhsNativeProcessor(paramsJson, fpsNumerator, fpsDenominator))
  {
      for (int frame = firstFrame; frame <= lastFrame; ++frame)
      {
          ReadSceneLinearRgba(frame, inputRgba); // width * height * 4 floats
          processor.Process(inputRgba, outputRgba, width, height, false);
          WriteSceneLinearRgba(frame, outputRgba);
      }
  }
  ```
- The package was compiled and run in Unity 6000.6.10f1 with URP 17.6.0 on
  Windows x86_64. HDRP and other Unity versions have not been runtime-checked.
