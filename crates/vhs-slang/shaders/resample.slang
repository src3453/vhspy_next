// 入力画像の読み込みに使う共通部品 (composite.slang と camera.slang から使う)
module resample;

/// 負と NaN を 0 にする (vhs-core の frame::sanitize)
public float3 sanitize(float3 v) { return float3(v.x > 0.0 ? v.x : 0.0, v.y > 0.0 ? v.y : 0.0, v.z > 0.0 ? v.z : 0.0); }

/// 縮小・拡大の重みの範囲 (vhs-core の frame::resample_weights と同じ三角フィルタ)。
/// 縮小ではフィルタの幅を縮小率まで広げて面積平均に近づける
public struct ResampleAxis
{
    public float center;
    public float support;
    public int lo;
    public int hi;

    public __init(int i, int src, int dst)
    {
        float scale = float(src) / float(dst);
        support = max(scale, 1.0);
        center = (float(i) + 0.5) * scale;
        lo = int(max(floor(center - support), 0.0));
        hi = min(int(ceil(center + support)), src);
    }

    public float weight(int j) { return max(1.0 - abs((float(j) + 0.5 - center) / support), 0.0); }
}
