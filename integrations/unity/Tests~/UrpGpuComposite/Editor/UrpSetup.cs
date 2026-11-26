using UnityEditor;
using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.Rendering.Universal;
using VhsNext.URP;

// Creates a URP asset whose renderer has the VHS Next renderer feature, and makes it active.
public static class UrpSetup
{
    public static void Run()
    {
        var data = ScriptableObject.CreateInstance<UniversalRendererData>();
        AssetDatabase.CreateAsset(data, "Assets/VhsRenderer.asset");
        var feature = ScriptableObject.CreateInstance<VhsNextRendererFeature>();
        feature.name = "VhsNext";
        AssetDatabase.AddObjectToAsset(feature, data);
        data.rendererFeatures.Add(feature);
        EditorUtility.SetDirty(data);

        var asset = UniversalRenderPipelineAsset.Create(data);
        asset.supportsHDR = true;
        asset.msaaSampleCount = 1;
        AssetDatabase.CreateAsset(asset, "Assets/VhsUrp.asset");
        GraphicsSettings.defaultRenderPipeline = asset;
        for (int i = 0; i < QualitySettings.names.Length; ++i)
        {
            QualitySettings.SetQualityLevel(i, false);
            QualitySettings.renderPipeline = asset;
        }
        AssetDatabase.SaveAssets();
        Debug.Log("URP setup done: " + GraphicsSettings.defaultRenderPipeline);
        EditorApplication.Exit(0);
    }
}
