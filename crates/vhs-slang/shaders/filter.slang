// 因果 IIR フィルタ (RBJ biquad と Butterworth の縦続接続)。crates/vhs-core/src/filter.rs の移植。
//
// 係数の式は Rust 側と同じ。GPU の sin/cos の精度差で係数はわずかにずれる。
// Cascade は段数をジェネリクスの定数にして、ループを展開させる (動的な配列添字を避ける)。
module filter;

import timing;

public struct Biquad
{
    public float b0, b1, b2, a1, a2;
    public float z1, z2;

    public static Biquad identity()
    {
        Biquad q;
        q.b0 = 1.0; q.b1 = 0.0; q.b2 = 0.0; q.a1 = 0.0; q.a2 = 0.0;
        q.z1 = 0.0; q.z2 = 0.0;
        return q;
    }

    static Biquad norm(float b0, float b1, float b2, float a0, float a1, float a2)
    {
        Biquad q;
        q.b0 = b0 / a0; q.b1 = b1 / a0; q.b2 = b2 / a0;
        q.a1 = a1 / a0; q.a2 = a2 / a0;
        q.z1 = 0.0; q.z2 = 0.0;
        return q;
    }

    static float2 w0(float fc, float fs)
    {
        float f = clamp(fc, 1e-6 * fs, 0.45 * fs);
        float w = 2.0 * PI * f / fs;
        return float2(cos(w), sin(w));
    }

    public static Biquad notchAt(float fc, float q, float fs)
    {
        float2 cs = w0(fc, fs);
        float alpha = cs.y / (2.0 * q);
        return norm(1.0, -2.0 * cs.x, 1.0, 1.0 + alpha, -2.0 * cs.x, 1.0 - alpha);
    }

    public static Biquad lowpassAt(float fc, float q, float fs)
    {
        float2 cs = w0(fc, fs);
        float alpha = cs.y / (2.0 * q);
        float c = cs.x;
        return norm((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    public static Biquad bandpassAt(float fc, float q, float fs)
    {
        float2 cs = w0(fc, fs);
        float alpha = cs.y / (2.0 * q);
        return norm(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cs.x, 1.0 - alpha);
    }

    /// 初期状態の影響が `eps` 倍まで減衰するサンプル数 (極の半径から求める)。
    /// チャンク分割で、チャンクの手前からゼロ状態で回し始めるウォームアップ長に使う
    public int decaySamples(float eps)
    {
        float disc = a1 * a1 - 4.0 * a2;
        float r;
        if (disc < 0.0)
        {
            r = sqrt(a2); // 複素共役の極
        }
        else
        {
            float sq = sqrt(disc);
            r = 0.5 * max(abs(-a1 + sq), abs(-a1 - sq));
        }
        r = clamp(r, 1e-6, 0.9999);
        return int(ceil(log(eps) / log(r)));
    }

    [mutating]
    public float process(float x)
    {
        float y = b0 * x + z1;
        z1 = b1 * x - a1 * y + z2;
        z2 = b2 * x - a2 * y;
        return y;
    }
}

/// biquad の N 段縦続。Butterworth の次数は 2N
public struct Cascade<let N : int>
{
    public Biquad s[N];

    public static Cascade<N> butterLowpassAt(float fc, float fs)
    {
        Cascade<N> c;
        [ForceUnroll]
        for (int k = 1; k <= N; k++)
        {
            float theta = PI * float(2 * k - 1) / float(4 * N);
            c.s[k - 1] = Biquad.lowpassAt(fc, 1.0 / (2.0 * cos(theta)), fs);
        }
        return c;
    }

    public static Cascade<N> single(Biquad q)
    {
        Cascade<N> c;
        [ForceUnroll]
        for (int k = 0; k < N; k++)
            c.s[k] = k == 0 ? q : Biquad.identity();
        return c;
    }

    /// 縦続全体の減衰サンプル数。段ごとの値の和を取る (重根で裾が延びるぶんも含めた安全側の見積もり)
    public int decaySamples(float eps)
    {
        int total = 0;
        [ForceUnroll]
        for (int k = 0; k < N; k++)
            total += s[k].decaySamples(eps);
        return total;
    }

    [mutating]
    public float process(float x)
    {
        float y = x;
        [ForceUnroll]
        for (int k = 0; k < N; k++)
            y = s[k].process(y);
        return y;
    }
}
