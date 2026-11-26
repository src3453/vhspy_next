Shader "Hidden/VhsNext/AspectFit"
{
    Properties
    {
        _MainTex ("Source", 2D) = "black" {}
        _VhsTargetAspect ("Target Aspect", Float) = 1.3333333
        _VhsOutputAspect ("Output Aspect", Float) = 1.7777778
        _VhsSourceAspect ("Source Aspect (0 = texture size)", Float) = 0
    }

    SubShader
    {
        Cull Off
        ZWrite Off
        ZTest Always

        Pass
        {
            HLSLPROGRAM
            #pragma vertex vert_img
            #pragma fragment frag
            #include "UnityCG.cginc"

            sampler2D _MainTex;
            float4 _MainTex_TexelSize;
            float _VhsTargetAspect;
            float _VhsOutputAspect;
            // The GPU path decodes into the 754×480 (PAL 922×576) raster, which stands for the
            // whole camera frame; it passes the camera aspect here instead of the texture size.
            float _VhsSourceAspect;

            float4 frag(v2f_img input) : SV_Target
            {
                float2 uv = input.uv;
                #if UNITY_UV_STARTS_AT_TOP
                if (_MainTex_TexelSize.y < 0.0)
                    uv.y = 1.0 - uv.y;
                #endif

                float targetAspect = max(_VhsTargetAspect, 0.01);
                float outputAspect = max(_VhsOutputAspect, 0.01);
                float sourceAspect = _VhsSourceAspect > 0.0
                    ? _VhsSourceAspect
                    : _MainTex_TexelSize.z / max(abs(_MainTex_TexelSize.w), 1.0);

                float2 apertureSize = float2(
                    min(1.0, targetAspect / outputAspect),
                    min(1.0, outputAspect / targetAspect));
                float2 apertureMin = (1.0 - apertureSize) * 0.5;
                float2 apertureMax = apertureMin + apertureSize;
                if (any(uv < apertureMin) || any(uv > apertureMax))
                    return float4(0.0, 0.0, 0.0, 1.0);

                float2 apertureUv = (uv - apertureMin) / apertureSize;
                float2 sourceScale = float2(
                    min(1.0, targetAspect / sourceAspect),
                    min(1.0, sourceAspect / targetAspect));
                float2 sourceUv = 0.5 + (apertureUv - 0.5) * sourceScale;
                float4 color = tex2D(_MainTex, sourceUv);
                color.a = 1.0;
                return color;
            }
            ENDHLSL
        }
    }
}
