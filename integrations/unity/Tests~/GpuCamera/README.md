# Editor check for the GPU tube camera

`Editor/VhsGpuCameraCheck.cs` compares `VhsGpuCamera` with the CPU camera
(`vhs_core::camera::CameraSim`). It also compares `VhsGpuComposite` with
`camera.enabled` against the CPU camera → encode → decode path. The input is a
scene-linear float HDR image with a 6× white highlight moving across 6 fields.
Preamp noise is off. The script checks:

1. The C# port of the per-tube spread and of the sensitivity and dark-current
   maps (`GenerateTubes` and `BuildMaps`) against `TubeMaps`.
2. The camera output after every field.
3. The decoded frame after the last field.

## Run

```powershell
cargo run -p vhs-slang --example dump_camera_reference --release -- $env:TEMP\camref_ntsc ntsc
```

```powershell
cargo run -p vhs-slang --example dump_camera_reference --release -- $env:TEMP\camref_pal pal
```

Copy `Editor/VhsGpuCameraCheck.cs` into the `Assets/Editor` folder of a project
that references this package. Then call `VhsGpuCameraCheck.Run(dir)` from an
editor script or the Unity MCP `RunCommand` tool. It returns the report and also
logs it.

## Results

Unity 6000.6.10f1, Direct3D 12, RX 7600:

| | NTSC | PAL |
|---|---|---|
| maps, max abs diff | 2.4e-7 | 2.4e-7 |
| camera, max over 6 fields | 0.012 LSB | 0.013 LSB |
| camera → composite, max / mean | 0.50 / 0.246 LSB | 0.50 / 0.246 LSB |
