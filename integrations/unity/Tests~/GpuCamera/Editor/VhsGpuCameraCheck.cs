using System;
using System.IO;
using System.Text;
using Unity.Collections;
using UnityEngine;
using UnityEngine.Rendering;
using VhsNext;

/// <summary>
/// Compares VhsGpuCamera (and VhsGpuComposite with the camera enabled) against CPU reference data
/// written by `cargo run -p vhs-slang --example dump_camera_reference --release -- <dir> [ntsc|pal]`.
/// Run from the editor with VhsGpuCameraCheck.Run(dir); it returns a report and logs it.
/// </summary>
public static class VhsGpuCameraCheck
{
    [Serializable]
    private sealed class Reference
    {
        public string standard = "ntsc";
        public ulong seed;
        public VhsGpuCamera.Settings camera = new VhsGpuCamera.Settings();
    }

    public static string Run(string dir)
    {
        var report = new StringBuilder();
        string[] info = File.ReadAllText(Path.Combine(dir, "info.txt")).Split(new[] { ' ', '\n', '\r' }, StringSplitOptions.RemoveEmptyEntries);
        int w = int.Parse(info[0]), h = int.Parse(info[1]), fields = int.Parse(info[2]);
        bool pal = info[3] == "Pal";
        string json = File.ReadAllText(Path.Combine(dir, "params.json"));
        var reference = JsonUtility.FromJson<Reference>(json);
        int n = w * h;
        report.AppendLine($"{info[3]} {w}x{h}, {fields} fields, seed {reference.seed}, on {SystemInfo.graphicsDeviceType} {SystemInfo.graphicsDeviceName}");

        // 1. The C# port of the tube spread and the sensitivity / dark maps against vhs-core's TubeMaps
        VhsGpuCamera.Tube[] tubes = VhsGpuCamera.GenerateTubes(reference.camera, reference.seed);
        VhsGpuCamera.BuildMaps(reference.camera, reference.seed, tubes, w, h, out float[][] sens, out float[][] dark);
        float[] maps = ReadFloats(Path.Combine(dir, "maps.bin"));
        float mapMax = 0f;
        for (int c = 0; c < 3; ++c)
            for (int i = 0; i < n; ++i)
            {
                mapMax = Math.Max(mapMax, Math.Abs(sens[c][i] - maps[c * n + i]));
                mapMax = Math.Max(mapMax, Math.Abs(dark[c][i] - maps[(3 + c) * n + i]));
            }
        report.AppendLine($"maps (C# port vs vhs-core): max abs diff {mapMax:G3}");

        var scenes = new Texture2D[fields];
        for (int f = 0; f < fields; ++f)
        {
            scenes[f] = new Texture2D(w, h, TextureFormat.RGBAFloat, false, true) { filterMode = FilterMode.Point };
            scenes[f].SetPixelData(ReadFloats(Path.Combine(dir, $"scene_{f}.bin")), 0);
            scenes[f].Apply(false, false);
        }

        // 2. The camera alone, every field, against CameraSim
        float camMax = 0f;
        double camMeanWorst = 0;
        using (var camera = new VhsGpuCamera(reference.camera, reference.seed, pal))
        {
            for (int f = 0; f < fields; ++f)
            {
                using (var cmd = new CommandBuffer { name = "VHS camera check" })
                {
                    camera.RecordField(cmd, scenes[f], (ulong)f, true);
                    Graphics.ExecuteCommandBuffer(cmd);
                }
                float[] got = Readback(camera.Output);
                float[] expect = ReadFloats(Path.Combine(dir, $"camera_{f}.bin"));
                double sum = 0;
                for (int i = 0; i < n; ++i)
                    for (int c = 0; c < 3; ++c)
                    {
                        float d = Math.Abs(got[i * 4 + c] - expect[i * 3 + c]) * 255f;
                        camMax = Math.Max(camMax, d);
                        sum += d;
                    }
                camMeanWorst = Math.Max(camMeanWorst, sum / (3.0 * n));
            }
        }
        report.AppendLine($"camera vs CPU, all {fields} fields: max {camMax:F3} LSB, worst field mean {camMeanWorst:F4} LSB");

        // 3. VhsGpuComposite with "camera.enabled" (camera → encode → decode), one field per Record
        float compMax = 0f;
        double compSum = 0;
        using (var composite = new VhsGpuComposite(json, pal ? 50u : 60000u, pal ? 1u : 1001u))
        {
            for (int f = 0; f < fields; ++f)
            {
                using (var cmd = new CommandBuffer { name = "VHS composite check" })
                {
                    composite.Record(cmd, scenes[f]);
                    Graphics.ExecuteCommandBuffer(cmd);
                }
            }
            float[] got = Readback(composite.Output);
            byte[] expect = File.ReadAllBytes(Path.Combine(dir, "composite.bin"));
            for (int i = 0; i < n; ++i)
                for (int c = 0; c < 3; ++c)
                {
                    // The GPU output is scene-linear; the CPU reference is display values
                    float display = Mathf.Pow(Mathf.Clamp01(got[i * 4 + c]), 1f / 2.2f) * 255f;
                    float d = Math.Abs(display - expect[i * 4 + c]);
                    compMax = Math.Max(compMax, d);
                    compSum += d;
                }
            report.AppendLine($"camera → composite vs CPU: max {compMax:F2} LSB, mean {compSum / (3.0 * n):F4} LSB, camera active {composite.Camera != null}");
        }

        foreach (Texture2D t in scenes)
            UnityEngine.Object.DestroyImmediate(t);
        string text = report.ToString();
        Debug.Log("[VhsGpuCameraCheck]\n" + text);
        return text;
    }

    private static float[] ReadFloats(string path)
    {
        byte[] bytes = File.ReadAllBytes(path);
        var floats = new float[bytes.Length / 4];
        Buffer.BlockCopy(bytes, 0, floats, 0, bytes.Length);
        return floats;
    }

    private static float[] Readback(RenderTexture texture)
    {
        AsyncGPUReadbackRequest request = AsyncGPUReadback.Request(texture, 0);
        request.WaitForCompletion();
        if (request.hasError)
            throw new InvalidOperationException("GPU readback failed");
        NativeArray<float> data = request.GetData<float>();
        return data.ToArray();
    }
}
