using System;
using System.Collections;
using System.IO;
using System.Reflection;
using NUnit.Framework;
using UnityEngine;
using UnityEngine.Experimental.Rendering;
using UnityEngine.Rendering;
using UnityEngine.TestTools;
using VhsNext;

// Renders a small unlit scene through URP twice: without the VHS effect (raw camera color) and
// with VhsUnityEffect in GpuComposite mode. Writes the raw frame, the GPU raster and the
// presented frame for comparison with the wgpu reference host.
public class VhsUrpTest
{
    const int Width = 1280, Height = 720;

    static string OutDir => Environment.GetEnvironmentVariable("VHS_URP_OUT");

    static void Quad(string name, Vector3 pos, Vector2 size, Color color, Shader shader)
    {
        var go = GameObject.CreatePrimitive(PrimitiveType.Quad);
        go.name = name;
        go.transform.position = pos;
        go.transform.localScale = new Vector3(size.x, size.y, 1);
        var mat = new Material(shader);
        mat.SetColor("_BaseColor", color);
        go.GetComponent<Renderer>().sharedMaterial = mat;
    }

    static float[] Read(Texture tex)
    {
        var req = AsyncGPUReadback.Request(tex, 0, TextureFormat.RGBAFloat);
        req.WaitForCompletion();
        Assert.IsFalse(req.hasError, "readback failed for " + tex.name);
        return req.GetData<float>().ToArray();
    }

    static void Save(string name, float[] data, int w, int h)
    {
        var bytes = new byte[data.Length * 4];
        Buffer.BlockCopy(data, 0, bytes, 0, bytes.Length);
        File.WriteAllBytes(Path.Combine(OutDir, name + ".bin"), bytes);
        File.WriteAllText(Path.Combine(OutDir, name + ".txt"), w + " " + h);
    }

    [UnityTest]
    public IEnumerator GpuCompositeThroughUrp()
    {
        Assert.IsNotNull(GraphicsSettings.defaultRenderPipeline, "URP asset not active");
        Debug.Log("VHS URP test on " + SystemInfo.graphicsDeviceType);
        Directory.CreateDirectory(OutDir);

        var unlit = Shader.Find("Universal Render Pipeline/Unlit");
        Assert.IsNotNull(unlit);
        // Colour bars across the middle, an orientation marker (red) at the top, green at the bottom left,
        // and fine vertical stripes to show the luma bandwidth.
        Color[] bars = { new Color(.75f,.75f,.75f), new Color(.75f,.75f,0), new Color(0,.75f,.75f), new Color(0,.75f,0),
                         new Color(.75f,0,.75f), new Color(.75f,0,0), new Color(0,0,.75f), Color.black };
        for (int i = 0; i < bars.Length; ++i)
            Quad("bar" + i, new Vector3(-7f + i * 2f, 0, 0), new Vector2(2f, 4f), bars[i].linear, unlit);
        Quad("top", new Vector3(0, 3.6f, 0), new Vector2(6f, 1.2f), Color.red.linear, unlit);
        Quad("bottomLeft", new Vector3(-5f, -3.6f, 0), new Vector2(3f, 1.2f), Color.green.linear, unlit);
        for (int i = 0; i < 12; ++i)
            Quad("stripe" + i, new Vector3(2f + i * 0.3f, -3.6f, -0.01f), new Vector2(0.15f, 1.2f), Color.white, unlit);

        var rt = new RenderTexture(Width, Height, 24, GraphicsFormat.R16G16B16A16_SFloat) { name = "camera target" };
        rt.Create();
        var camGo = new GameObject("Camera");
        camGo.SetActive(false);
        var cam = camGo.AddComponent<Camera>();
        cam.transform.position = new Vector3(0, 0, -10);
        cam.orthographic = true;
        cam.orthographicSize = 4.5f;
        cam.clearFlags = CameraClearFlags.SolidColor;
        cam.backgroundColor = new Color(0.2f, 0.2f, 0.25f).linear;
        cam.allowHDR = true;
        cam.allowMSAA = false;
        cam.targetTexture = rt;
        camGo.SetActive(true);

        // 1) Raw camera colour, no VHS effect.
        for (int i = 0; i < 5; ++i)
            yield return null;
        Save("raw", Read(rt), Width, Height);

        // 2) Same camera with the GPU composite backend.
        camGo.SetActive(false);
        var effect = camGo.AddComponent<VhsUnityEffect>();
        typeof(VhsUnityEffect).GetField("backend", BindingFlags.NonPublic | BindingFlags.Instance)
            .SetValue(effect, VhsUnityEffect.ProcessingBackend.GpuComposite);
        camGo.SetActive(true);
        for (int i = 0; i < 40; ++i)
            yield return null;
        Debug.Log("VHS URP status: " + effect.Status);
        StringAssert.StartsWith("Processing (GPU", effect.Status);

        var gpu = (VhsGpuComposite)typeof(VhsUnityEffect).GetField("_gpu", BindingFlags.NonPublic | BindingFlags.Instance).GetValue(effect);
        ulong fields = (ulong)typeof(VhsGpuComposite).GetField("_fieldIndex", BindingFlags.NonPublic | BindingFlags.Instance).GetValue(gpu);
        Save("gpu_raster", Read(gpu.Output), gpu.Output.width, gpu.Output.height);
        Save("presented", Read(rt), Width, Height);
        File.WriteAllText(Path.Combine(OutDir, "fields.txt"), fields.ToString());
        Debug.Log("VHS URP test wrote outputs, fields " + fields);

        // GPU / CPU time of the "VHS Next" render-graph pass over 120 frames.
        var rec = UnityEngine.Profiling.Recorder.Get("VHS Next");
        rec.enabled = true;
        var gpuMs = new System.Collections.Generic.List<double>();
        var cpuMs = new System.Collections.Generic.List<double>();
        for (int i = 0; i < 120; ++i)
        {
            yield return null;
            if (rec.isValid && rec.gpuSampleBlockCount > 0)
                gpuMs.Add(rec.gpuElapsedNanoseconds * 1e-6);
            if (rec.isValid && rec.sampleBlockCount > 0)
                cpuMs.Add(rec.elapsedNanoseconds * 1e-6);
        }
        gpuMs.Sort();
        cpuMs.Sort();
        Debug.Log(string.Format("VHS URP timing: valid {0}, gpu samples {1}, gpu median {2:F3} ms, cpu median {3:F3} ms",
            rec.isValid, gpuMs.Count, gpuMs.Count > 0 ? gpuMs[gpuMs.Count / 2] : -1, cpuMs.Count > 0 ? cpuMs[cpuMs.Count / 2] : -1));
    }
}
