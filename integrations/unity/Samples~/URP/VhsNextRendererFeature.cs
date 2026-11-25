using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.Rendering.RenderGraphModule;
using UnityEngine.Rendering.Universal;

namespace VhsNext.URP
{
    /// <summary>
    /// Add to a URP Renderer and attach VhsUnityEffect to the camera to process.
    /// Uses Render Graph so it works with current Unity 6 URP versions.
    /// </summary>
    public sealed class VhsNextRendererFeature : ScriptableRendererFeature
    {
        [SerializeField] private RenderPassEvent injectionPoint = RenderPassEvent.BeforeRenderingPostProcessing;
        private VhsNextPass _pass;

        public override void Create()
        {
            _pass = new VhsNextPass { renderPassEvent = injectionPoint };
            _pass.requiresIntermediateTexture = true;
        }

        public override void AddRenderPasses(ScriptableRenderer renderer, ref RenderingData renderingData)
        {
            Camera camera = renderingData.cameraData.camera;
            if (camera == null || !camera.TryGetComponent(out VhsUnityEffect effect) || !effect.isActiveAndEnabled)
                return;
            _pass.SetEffect(effect);
            renderer.EnqueuePass(_pass);
        }

        private sealed class VhsNextPass : ScriptableRenderPass
        {
            private sealed class PassData
            {
                internal TextureHandle Source;
                internal TextureHandle Destination;
                internal VhsUnityEffect Effect;
            }

            private VhsUnityEffect _effect;

            internal void SetEffect(VhsUnityEffect effect)
            {
                _effect = effect;
            }

            private static void ExecutePass(PassData data, UnsafeGraphContext context)
            {
                CommandBuffer commandBuffer = CommandBufferHelpers.GetNativeCommandBuffer(context.cmd);
                Texture source = data.Source;
                RenderTexture destination = data.Destination;
                data.Effect.Record(commandBuffer, source, destination);
            }

            public override void RecordRenderGraph(RenderGraph renderGraph, ContextContainer frameData)
            {
                if (_effect == null)
                    return;

                UniversalResourceData resourceData = frameData.Get<UniversalResourceData>();
                if (resourceData.isActiveTargetBackBuffer)
                    return;

                TextureHandle source = resourceData.activeColorTexture;
                var destinationDescriptor = renderGraph.GetTextureDesc(source);
                destinationDescriptor.name = "VHS Next Camera Color";
                destinationDescriptor.clearBuffer = false;
                destinationDescriptor.msaaSamples = MSAASamples.None;
                TextureHandle destination = renderGraph.CreateTexture(destinationDescriptor);

                using (var builder = renderGraph.AddUnsafePass<PassData>("VHS Next", out var passData))
                {
                    passData.Source = source;
                    passData.Destination = destination;
                    passData.Effect = _effect;
                    builder.UseTexture(source, AccessFlags.Read);
                    builder.UseTexture(destination, AccessFlags.WriteAll);
                    builder.SetRenderFunc(static (PassData data, UnsafeGraphContext context) => ExecutePass(data, context));
                }

                resourceData.cameraColor = destination;
            }
        }
    }
}
