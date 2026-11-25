using UnityEngine.Rendering;
using UnityEngine.Rendering.HighDefinition;

namespace VhsNext.HDRP
{
    /// <summary>
    /// Add this pass to an HDRP Custom Pass Volume and attach VhsUnityEffect to its camera.
    /// </summary>
    public sealed class VhsNextCustomPass : CustomPass
    {
        protected override void Execute(CustomPassContext context)
        {
            if (context.hdCamera == null || context.hdCamera.camera == null)
                return;

            VhsUnityEffect effect = context.hdCamera.camera.GetComponent<VhsUnityEffect>();
            if (effect == null || !effect.isActiveAndEnabled)
                return;

            var colorTarget = context.cameraColorBuffer;
            if (colorTarget == null || colorTarget.rt == null)
                return;

            effect.Record(context.cmd, colorTarget.rt);
        }
    }
}
