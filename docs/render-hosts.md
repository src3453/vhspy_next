# Render-host integration

The first host bridge is a small C ABI in `vhs-ffi`. It accepts a scene-linear
`float32 RGBA` frame, keeps a `vhs_core::Pipeline` alive between frames, and
returns a float buffer with the same dimensions. This is suitable for a
Blender Python add-on using `ctypes`, and avoids converting the render result
to an 8-bit file before it enters the VHS simulation.

## Build

```powershell
cargo build -p vhs-ffi --release
```

On Windows the library is `target/release/vhs_ffi.dll`. Other platforms produce
the matching shared-library extension for that platform.

## Frame contract

- Input is tightly packed RGBA `float32`, with scene-linear Rec.709/sRGB
  primaries and alpha in 0..1. `alpha_mode=0` means straight alpha;
  `alpha_mode=1` means premultiplied.
- Pass one frame at a time in increasing render order. The processor keeps tape,
  decoder, and field-clock state between calls. Create a fresh processor when a
  render sequence or simulation state should reset.
- The rational frame rate at construction determines how many NTSC/PAL fields
  advance for each host frame. For example, 24 fps against NTSC alternates
  between two and three fields per rendered frame.
- Output is float32 RGBA with the input dimensions and alpha. RGB is converted
  back to scene-linear values after the existing decoder output. The current
  decoder still resolves to 8-bit video internally, so float buffers preserve
  the host interface and HDR input path, but do not add precision to decoded
  VHS output.
- The core samples at its NTSC/PAL image size and the bridge resizes the decoded
  image back to the host frame dimensions.

The exported functions and status values are declared in
`crates/vhs-ffi/src/lib.rs`. `vhs_processor_new` accepts the same JSON `Params`
shape used by the desktop app. `vhs_processor_process_rgba_f32` returns 0 on
success and a negative status on invalid arguments or buffer sizes. Input and
output must be separate buffers.

## Blender add-on

The first Blender adapter is `integrations/blender/vhs_next_render_pass.py`.
1. Build the native library with `cargo build -p vhs-ffi --release`.
2. In Blender 4.2+, install the `.py` file from **Edit → Preferences →
   Get Extensions** using **Install Legacy Add-on** / **Install from Disk**,
   then enable the add-on.
3. Set the native-library path under **Render Properties → VHS Next**. The
   parameters file is optional; an empty path uses core defaults.

**Process Current Render Result** creates or updates a float image named
`VHS Pass`. Enable **Process animation renders** to handle each written frame
in render order and write 32-bit RGBA OpenEXR files to the selected folder
(the render frame step must be 1). The handler keeps one native processor alive
for the sequence and resets it if frame order is interrupted. The generated
image can be viewed in Blender or loaded into a compositor as an image input.
This first adapter runs after Blender produces a Render Result; it does not
evaluate as a live compositor node during that same render.
