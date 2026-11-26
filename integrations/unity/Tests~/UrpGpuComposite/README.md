# URP check for the GPU composite backend

This PlayMode test renders an unlit test scene through URP 17.6 with
`VhsNextRendererFeature`. It renders once without the effect and once with
`VhsUnityEffect` set to **Gpu Composite**. It then writes these files:

- `raw`: the camera color without the effect (scene-linear, 1280×720 half float)
- `gpu_raster`: `VhsGpuComposite.Output`, the 754×480 decoded raster
- `presented`: the camera target after the 4:3 aspect fit
- `fields.txt`: the number of fields processed

The test also logs the GPU and CPU time of the `VHS Next` render-graph pass.

## Run

1. Create an empty Unity 6000.6 project in a short path, for example
   `%TEMP%\vurp`. A deep path breaks URP's package import on Windows because
   of path-length limits.
2. Copy `manifest.json` to `Packages/`, and replace `REPO` with this
   repository's path.
3. Copy `Editor/` and `Tests/` to `Assets/`, and copy `Samples~/URP` to
   `Assets/VhsSamples/URP`.
4. Run the following commands:

```powershell
& "C:\Program Files\Unity\Hub\Editor\6000.6.10f1\Editor\Unity.exe" -batchmode -projectPath $env:TEMP\vurp -executeMethod UrpSetup.Run -logFile setup.log
```

```powershell
$env:VHS_URP_OUT = "$env:TEMP\vurp-out"; & "C:\Program Files\Unity\Hub\Editor\6000.6.10f1\Editor\Unity.exe" -batchmode -force-d3d11 -projectPath $env:TEMP\vurp -runTests -testPlatform PlayMode -testResults results.xml -logFile test.log
```

```powershell
cargo run -p vhs-slang --example replay --release -- $env:TEMP\vurp-out\raw.bin 1280 720 (Get-Content $env:TEMP\vurp-out\fields.txt) $env:TEMP\vurp-out\wgpu_raster.bin
```

```powershell
python integrations\unity\Tests~\UrpGpuComposite\compare_urp.py $env:TEMP\vurp-out
```

## Result (2026-11-26, Unity 6000.6.10f1, RX 7600)

| | D3D11 | D3D12 | Vulkan |
|---|---|---|---|
| Unity raster vs wgpu replay of the same camera frame | 0.00 LSB | 0.00 LSB | 0.00 LSB |
| Side bars | black | black | black |
| Presented vs raw camera | same orientation | same orientation | same orientation |
| `VHS Next` pass GPU time per frame (median) | 0.47 ms | not reported | 0.50 ms |
| `VHS Next` pass CPU time per frame (median) | 0.03 ms | 0.04 ms | 0.05 ms |

The GPU time covers one field, the 1280×720 resample, and the aspect-fit
blit at 60 fps. Under D3D12, Unity's `Recorder` returns no GPU time for this
pass.
