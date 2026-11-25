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
4. Add `VhsUnityEffect` to the camera to process. Assign an optional VHS Next
   Params JSON `TextAsset`; the empty field uses the core defaults. The default
   clock is 60/1 frames per second. Set it to match the source camera rate.
   `Output Aspect Width` and `Output Aspect Height` default to 4 and 3.

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
