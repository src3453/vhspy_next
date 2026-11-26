// コンポジット信号のエンコード → 受像機デコードの GPU 版 (PoC)。
//
// crates/vhs-core の encoder.rs / decoder.rs のうち、劣化のない経路だけを移植している:
// - エンコーダ: RGB → Y/I/Q (PAL は Y/U/V) → 帯域制限 → 直交変調 + 同期 + バースト
// - 受像機: 水平 AFC → Y/C 分離 (ノッチ / ラインくし形) → バースト検出 → 副搬送波 PLL・PAL アイデント
//   → 同期検波 → (PAL 遅延線) → RGB
//
// 1 フィールドぶんを次の順にディスパッチする (どれも前のカーネルの結果を読む):
//   vhsPrepare → vhsEncode → vhsSyncAfc → vhsBurstLock → vhsSeparateDemod → vhsResolve
//
// 並列化の方針:
// - ラインの中の因果 IIR はチャンク (ENCODE_CHUNK / DEMOD_CHUNK サンプル) に分け、1 チャンク = 1 スレッドで回す。
//   各スレッドはチャンクの手前からゼロ状態で回し始め (ウォームアップ)、初期状態の影響が WARMUP_EPS 倍に
//   減衰してから書き出す。ウォームアップ長はフィルタの極の半径から求める ([`Biquad.decaySamples`])。
//   ライン先頭のチャンクは CPU 版と同じくサンプル 0 からゼロ状態で始まるので厳密に一致する。
// - フィールド全体で逐次の処理 (水平 AFC、副搬送波 PLL) は 1 ワークグループのカーネルにし、
//   ラインごとの検出は全スレッドで並列に、逐次の漸化式だけを groupshared の上で 1 スレッドが回す。
// - AFC 補正後の信号はバッファに持たず、読むときに線形補間でずらす。
module composite;

import timing;
import filter;
import resample;

/// ホストから渡すパラメータ。4 バイトのスカラーだけを並べている (C# / Rust 側も同じ順で詰める)。
/// 定数バッファではなく 1 要素の構造化バッファで渡す: Unity は cbuffer の中の構造体を扱えないため。
public struct CompositeParams
{
    // --- フィールド ---
    public uint standard;     // 0 = NTSC, 1 = PAL
    public uint parity;       // 0 = 第 1 フィールド (偶数行), 1 = 第 2 フィールド
    public uint frameNo;      // フレーム番号 (下位ビットだけ使う)
    public uint fieldIndex;   // 通算フィールド番号 (下位 32 ビット)
    // --- 入力 ---
    public uint srcWidth;
    public uint srcHeight;
    public uint inputLinear;  // 1: 入力はシーンリニア (1/2.2 乗で表示用の値に戻す)
    public uint ycMode;       // 0 = ノッチ, 1 = ラインくし形
    // --- エンコーダ ---
    public float lumaBwMhz;
    public float iBwMhz;
    public float qBwMhz;
    public float uvBwMhz;
    public float chromaGain;
    // --- 受像機 ---
    public float chromaBwMhz;
    public float hPll;
    public float burstPll;
    public float contrast;
    public float brightness;
    public float saturation;
    public float hueDeg;
    public uint acc;
    public uint colorKiller;
    public uint palDelayLine;
    public uint outputLinear; // 1: 出力をシーンリニア (2.2 乗) にする
    public float vcxoOffsetHz; // 受像機の色副搬送波発振器の周波数ずれ [Hz]
}

/// ラインごとの中間結果
public struct LineInfo
{
    public float syncEdge;   // 検出した同期の後縁 (なければ負)
    public float correction; // AFC の補正量 [サンプル]
    public float zr;         // バースト相関
    public float zi;
    public float amp;        // バースト包絡線のピーク
    public float phase;      // PLL の基準位相 [rad]
    public float sv;         // PAL スイッチ
    public float gain;       // 色差のゲイン
}

/// エンコーダの 1 スレッドが受け持つサンプル数 (ホストのディスパッチ数もこれに合わせる)
public static const int ENCODE_CHUNK = 64;
/// 受像機の Y/C 分離・同期検波の 1 スレッドが受け持つ有効映像のサンプル数
public static const int DEMOD_CHUNK = 64;
/// フィールド単位の逐次処理のワークグループの大きさ。PAL の 288 ラインを 1 周で受け持てる大きさにする
/// (WebGPU の既定の上限 256 を超えるので、wgpu ではアダプタの上限を要求する)
public static const int SERIAL_THREADS = 320;
/// ウォームアップで初期状態の影響をどこまで小さくするか
static const float WARMUP_EPS = 1e-6;

/// [0] だけ使う
StructuredBuffer<CompositeParams> paramBuffer;

/// 入力画像 (RGB、srcWidth × srcHeight)。vhsPrepare が有効映像の大きさへ縮小・拡大して読む
Texture2D<float4> srcImage;
/// 出力画像 (activeWidth × 2·fieldLines)。このフィールドの行だけ書く
[format("rgba32f")]
WTexture2D<float4> dstImage;

/// 帯域制限前の映像信号 (フレームの全ライン = 2·fieldLines × activeWidth)。
/// xyz = Y [IRE]、cos 側 (NTSC は I、PAL は PAL スイッチを掛ける前の V)、sin 側 (Q / U)
RWStructuredBuffer<float4> video;
/// コンポジット信号 (fieldLines × lineSamples)
RWStructuredBuffer<float> signal;
/// Y/C 分離後の輝度 (fieldLines × activeWidth、有効映像だけ)
RWStructuredBuffer<float> luma;
/// 同期検波後の色差 (fieldLines × activeWidth)。x = cos 側 (I / ±V)、y = sin 側 (Q / U)
RWStructuredBuffer<float2> detected;
RWStructuredBuffer<LineInfo> lines;
/// フィールドをまたぐ状態: [0] = AFC の同期後縁位置, [1] = 初期化済みの印
RWStructuredBuffer<float> state;

static const float STATE_MAGIC = 12345.0;

CompositeParams params() { return paramBuffer[0]; }

Standard currentStandard() { return standardFor(params().standard); }

int ceilDiv(int a, int b) { return (a + b - 1) / b; }

// ---------------------------------------------------------------------------
// エンコーダ
// ---------------------------------------------------------------------------

/// vhsPrepare の 1 ワークグループが受け持つ出力のタイル (TILE_W 画素 × TILE_R 行)
public static const int PREPARE_TILE_W = 32;
public static const int PREPARE_TILE_R = 4;
/// タイルの入力の範囲を置く groupshared の大きさ (float3 で 15 KB。WebGPU の既定の上限 16 KB に収める)。
/// 1920×1080 → 754×480 ではおよそ 86 × 14 画素。これより大きな縮小率では直接読む経路になる
static const int PREPARE_TILE_TEXELS = 1280;
groupshared float3 gTile[PREPARE_TILE_TEXELS];

float3 loadSource(int x, int y, bool linear)
{
    float3 c = srcImage.Load(int3(x, y, 0)).rgb;
    return linear ? sanitize(c) : c;
}

/// 入力の画素を有効映像の大きさ (activeWidth × 2·fieldLines) に合わせて読み、帯域制限前の Y / 色差 [IRE] にする。
///
/// 入力 1 枚につき 1 回だけ、両フィールドの行をまとめて作る (ホストは入力を差し替えたときだけ
/// このカーネルを流し、フィールドごとのカーネルはここで作った値を読む)。
/// エンコーダのウォームアップで同じ画素を何度も変換しないためでもある。
///
/// 縮小は 1 出力画素あたり数十回の読み込みになり、テクスチャの読み込みで律速される。
/// ワークグループのタイルが使う入力の範囲を 1 回だけ groupshared に読み、隣り合う出力画素で使い回す
[shader("compute")]
[numthreads(PREPARE_TILE_W, PREPARE_TILE_R, 1)]
void vhsPrepare(uint3 gid : SV_GroupID, uint3 gtid : SV_GroupThreadID)
{
    Standard s = currentStandard();
    int2 size = int2(int(params().srcWidth), int(params().srcHeight));
    int2 dst = int2(s.activeWidth, 2 * s.fieldLines);
    bool linear = params().inputLinear != 0;
    int x0 = int(gid.x) * PREPARE_TILE_W;
    int r0 = int(gid.y) * PREPARE_TILE_R;
    int x = x0 + int(gtid.x);
    int row = r0 + int(gtid.y);
    bool inside = x < dst.x && row < dst.y;
    float3 rgb = float3(0.0);

    if (all(size == dst))
    {
        if (!inside)
            return;
        rgb = loadSource(x, row, linear);
    }
    else
    {
        // LinearFrame::resized と同じ分離可能な三角フィルタ (縦横それぞれ重みの和で正規化)
        ResampleAxis ax = ResampleAxis(min(x, dst.x - 1), size.x, dst.x);
        ResampleAxis ay = ResampleAxis(min(row, dst.y - 1), size.y, dst.y);
        // タイル全体が使う入力の範囲 (重みの範囲は出力の位置に対して単調)
        int tx0 = ResampleAxis(x0, size.x, dst.x).lo;
        int tx1 = ResampleAxis(min(x0 + PREPARE_TILE_W, dst.x) - 1, size.x, dst.x).hi;
        int ty0 = ResampleAxis(r0, size.y, dst.y).lo;
        int ty1 = ResampleAxis(min(r0 + PREPARE_TILE_R, dst.y) - 1, size.y, dst.y).hi;
        int tw = tx1 - tx0;
        int texels = tw * (ty1 - ty0);
        // タイルごとに一様な条件なので、バリアをこの中に置ける
        bool cached = texels <= PREPARE_TILE_TEXELS;
        if (cached)
        {
            int flat = int(gtid.y) * PREPARE_TILE_W + int(gtid.x);
            for (int k = flat; k < texels; k += PREPARE_TILE_W * PREPARE_TILE_R)
                gTile[k] = loadSource(tx0 + k % tw, ty0 + k / tw, linear);
            GroupMemoryBarrierWithGroupSync();
        }
        if (!inside)
            return;

        float3 acc = float3(0.0);
        float wsumX = 0.0;
        float wsumY = 0.0;
        for (int j = ax.lo; j < ax.hi; j++)
            wsumX += ax.weight(j);
        for (int i = ay.lo; i < ay.hi; i++)
        {
            float wy = ay.weight(i);
            if (wy <= 0.0)
                continue;
            wsumY += wy;
            float3 rowAcc = float3(0.0);
            for (int j = ax.lo; j < ax.hi; j++)
            {
                float3 c = cached ? gTile[(i - ty0) * tw + (j - tx0)] : loadSource(j, i, linear);
                rowAcc += c * ax.weight(j);
            }
            acc += rowAcc * wy;
        }
        rgb = acc / max(wsumX * wsumY, 1e-20);
    }
    if (linear)
    {
        // vhs-core の VideoFrame::copy_from_linear と同じ表示用の値への変換
        rgb = pow(min(rgb, float3(1.0)), float3(1.0 / 2.2));
    }

    Levels lv = s.ire;
    float span = lv.span();
    float y = lv.black + span * dot(rgb, float3(0.299, 0.587, 0.114));
    float i;
    float q;
    if (s.pal)
    {
        // PAL スイッチ (V の符号) はフィールドとラインで決まるので、エンコーダで掛ける
        i = span * dot(rgb, float3(0.615, -0.515, -0.100));
        q = span * dot(rgb, float3(-0.147, -0.289, 0.436));
    }
    else
    {
        i = span * dot(rgb, float3(0.5959, -0.2746, -0.3213));
        q = span * dot(rgb, float3(0.2115, -0.5227, 0.3112));
    }
    video[row * s.activeWidth + x] = float4(y, i, q, 0.0);
}

/// スレッド = (ライン, ENCODE_CHUNK サンプルのチャンク)
[shader("compute")]
[numthreads(64, 1, 1)]
void vhsEncode(uint3 tid : SV_DispatchThreadID)
{
    Standard s = currentStandard();
    int chunks = ceilDiv(s.lineSamples, ENCODE_CHUNK);
    int l = int(tid.x) / chunks;
    int chunk = int(tid.x) % chunks;
    if (l >= s.fieldLines)
        return;

    Levels lv = s.ire;
    float aBw = s.pal ? params().uvBwMhz : params().iBwMhz;
    float bBw = s.pal ? params().uvBwMhz : params().qBwMhz;
    var yLp = Cascade<2>.butterLowpassAt(params().lumaBwMhz * 1e6, s.fs);
    var iLp = Cascade<1>.butterLowpassAt(aBw * 1e6, s.fs);
    var qLp = Cascade<1>.butterLowpassAt(bBw * 1e6, s.fs);
    int warmup = max(yLp.decaySamples(WARMUP_EPS), max(iLp.decaySamples(WARMUP_EPS), qLp.decaySamples(WARMUP_EPS)));
    // I/Q (U/V) のフィルタの遅れに合わせ、色差は群遅延のぶん先の画素から読む (実機の Y/C 遅延の整合)
    float yDelay = yLp.groupDelay(1e3, s.fs);
    int iLead = int(max(round(iLp.groupDelay(1e3, s.fs) - yDelay), 0.0));
    int qLead = int(max(round(qLp.groupDelay(1e3, s.fs) - yDelay), 0.0));
    int n0 = chunk * ENCODE_CHUNK;
    int n1 = min(n0 + ENCODE_CHUNK, s.lineSamples);
    int start = max(0, n0 - warmup);

    // NTSC は I/Q 軸 (U/V から 33°)、PAL は U/V 軸
    float rot = s.pal ? 0.0 : 33.0 * PI / 180.0;
    int videoRow = (2 * l + int(params().parity)) * s.activeWidth - s.activeStart;
    int base = s.linePhaseBase(l, params().frameNo, params().parity);
    float sv = s.vSwitch(l, params().frameNo, params().parity);
    float burstHalf = lv.burst * FRAC_1_SQRT_2;
    int activeEnd = s.activeEnd();
    int offset = l * s.lineSamples;
    // 4fsc では副搬送波の位相は 90° 刻みなので、cos/sin は 4 象限ぶんの表で足りる
    float4 cosIq;
    float4 sinIq;
    [ForceUnroll]
    for (int k = 0; k < 4; k++)
    {
        float a = float(k) * FRAC_PI_2 + rot;
        cosIq[k] = cos(a);
        sinIq[k] = sin(a);
    }

    for (int n = start; n < n1; n++)
    {
        float y = lv.blank;
        float i = 0.0;
        float q = 0.0;
        if (n < s.syncEnd)
        {
            y = lv.sync;
        }
        else
        {
            if (n >= s.activeStart && n < activeEnd)
                y = video[videoRow + n].x;
            int ni = n + iLead;
            if (ni >= s.activeStart && ni < activeEnd)
                i = sv * video[videoRow + ni].y; // PAL スイッチ (NTSC は sv = 1)
            int nq = n + qLead;
            if (nq >= s.activeStart && nq < activeEnd)
                q = video[videoRow + nq].z;
        }
        y = yLp.process(y);
        i = iLp.process(i);
        q = qLp.process(q);
        if (n < n0)
            continue; // ウォームアップ

        int k = (n + base) & 3;
        float c = params().chromaGain * (q * sinIq[k] + i * cosIq[k]);
        if (n >= s.burstStart && n < s.burstEnd)
        {
            if (s.pal)
                c += burstHalf * (sv * cos4(k) - sin4(k)); // -U 軸から V 側へ ±45°
            else
                c -= lv.burst * sin4(k);
        }
        signal[offset + n] = y + c;
    }
}

// ---------------------------------------------------------------------------
// 受像機
// ---------------------------------------------------------------------------

/// 同期の後縁 (sync/2 を下から上へ横切る位置) を探す。左右対称の移動平均でならしてからスライスする
float findSyncEdge(int l, Standard s)
{
    static const int R = 4;
    int o = l * s.lineSamples;
    float thr = s.ire.sync / 2.0;
    float acc = 0.0;
    for (int k = 0; k < 2 * R + 1; k++)
        acc += signal[o + k];
    float normK = 1.0 / float(2 * R + 1);
    float prev = acc * normK;
    int lowRun = 0;
    for (int n = R + 1; n < s.activeStart - 10; n++)
    {
        acc += signal[o + n + R] - signal[o + n - R - 1];
        float cur = acc * normK;
        lowRun = prev < thr ? lowRun + 1 : 0;
        // 同期パルス幅の 1/4 以上低レベルが続いたあとの立ち上がりだけを採る
        if (lowRun >= 16 && prev < thr && cur >= thr)
        {
            float t = (thr - prev) / (cur - prev);
            return float(n) - 1.0 + t;
        }
        prev = cur;
    }
    return -1.0;
}

// フィールド単位のカーネルの作業領域 (スレッド t がライン t を受け持つ。SERIAL_THREADS はどの方式のライン数より大きい)
groupshared float gA[SERIAL_THREADS];
groupshared float gB[SERIAL_THREADS];
groupshared float gC[SERIAL_THREADS];
groupshared float gPos0;

/// 水平 AFC: 各ラインの同期を並列に検出し、時定数付きの追従 (フィールドをまたいで状態を持つ) を求める。
///
/// 追従の漸化式 pos ← pos + k·(edge − pos) は、ラインごとのアフィン写像 pos ← a·pos + b
/// (同期が見つかったラインは a = 1 − k, b = k·edge、ないラインは恒等) の合成なので、
/// groupshared 上の並列スキャン (Hillis–Steele) で全ラインの位置を一度に出せる
[shader("compute")]
[numthreads(SERIAL_THREADS, 1, 1)]
void vhsSyncAfc(uint3 gtid : SV_GroupThreadID)
{
    Standard s = currentStandard();
    int t = int(gtid.x);
    if (t == 0)
        gPos0 = state[1] == STATE_MAGIC ? state[0] : float(s.syncEnd);
    float k = clamp(params().hPll, 0.0, 1.0);
    float a = 1.0;
    float b = 0.0;
    if (t < s.fieldLines)
    {
        float edge = findSyncEdge(t, s);
        lines[t].syncEdge = edge;
        if (edge >= 0.0)
        {
            a = 1.0 - k;
            b = k * edge;
        }
    }
    gA[t] = a;
    gB[t] = b;
    GroupMemoryBarrierWithGroupSync();
    for (int offset = 1; offset < SERIAL_THREADS; offset *= 2)
    {
        if (t >= offset)
        {
            // 手前の区間の写像 (pa, pb) のあとに自分の区間の写像 (a, b) を掛ける
            float pa = gA[t - offset];
            float pb = gB[t - offset];
            b = a * pb + b;
            a = a * pa;
        }
        GroupMemoryBarrierWithGroupSync();
        gA[t] = a;
        gB[t] = b;
        GroupMemoryBarrierWithGroupSync();
    }
    if (t < s.fieldLines)
    {
        float pos = a * gPos0 + b;
        lines[t].correction = pos - float(s.syncEnd);
        if (t == s.fieldLines - 1)
        {
            state[0] = pos;
            state[1] = STATE_MAGIC;
        }
    }
}

/// AFC 補正後の信号を読むための、ラインごとの小数サンプルのずれ (filter.rs の fractional_shift)
struct LineShift
{
    int offset; // ライン先頭のバッファ位置
    int i0;
    float frac;
    int len;
    float fill;

    __init(int l, Standard s)
    {
        offset = l * s.lineSamples;
        len = s.lineSamples;
        fill = s.ire.blank;
        float shift = -lines[l].correction;
        float f = floor(shift);
        frac = shift - f;
        i0 = int(f);
    }

    float at(int n)
    {
        int k = n - i0;
        float a = (k >= 0 && k < len) ? signal[offset + k] : fill;
        float b = (k - 1 >= 0 && k - 1 < len) ? signal[offset + k - 1] : fill;
        return a + (b - a) * frac;
    }
}

/// くし形の相手: 副搬送波が 180° ずれたライン (NTSC は 1H、PAL は V の符号も揃う 2H 前)
int combLag(Standard s) { return s.pal ? 2 : 1; }

Biquad chromaBandpass(Standard s) { return Biquad.bandpassAt(s.fsc, 1.5, s.fs); }

float wrapAngle(float a) { return a - TAU * round(a / TAU); }

/// バースト検出と副搬送波 PLL・PAL アイデント。
/// バーストの検出と PAL アイデントの多数決は並列 (多数決は groupshared の木構造の総和)。
/// PLL はラインごとに即時同期 (burstPll = 1、既定) なら並列、追従係数が 1 未満なら折り返しを含む
/// 非線形の漸化式になるので 1 スレッドで逐次に回す
[shader("compute")]
[numthreads(SERIAL_THREADS, 1, 1)]
void vhsBurstLock(uint3 gtid : SV_GroupThreadID)
{
    Standard s = currentStandard();
    int t = int(gtid.x);
    int count = s.fieldLines;
    bool active = t < count;
    float step = float(s.linePhaseStep()) * FRAC_PI_2;

    // --- バーストの位相・振幅 (ゲートの手前からウォームアップしてバンドパスを回す) ---
    float mu = 0.0;
    float amp = 0.0;
    if (active)
    {
        int l = t;
        int lag = combLag(s);
        bool useComb = params().ycMode == 1 && l >= lag;
        var bp = Cascade<1>.single(chromaBandpass(s));
        int gateEnd = s.burstEnd + 16;
        int start = max(0, s.burstStart - bp.decaySamples(WARMUP_EPS));
        LineShift cur = LineShift(l, s);
        LineShift prev = LineShift(useComb ? l - lag : l, s);
        float zr = 0.0;
        float zi = 0.0;
        float cr = 0.0;
        float ci = 0.0;
        for (int n = start; n < gateEnd; n++)
        {
            float x = cur.at(n);
            float c = bp.process(useComb ? 0.5 * (x - prev.at(n)) : x);
            if (n < s.burstStart)
                continue;
            cr += c * cos4(n & 3);
            ci -= c * sin4(n & 3);
            // 1 周期 (4 サンプル) ごとに包絡線を取る。ゲートの始まりは 4 の倍数とは限らないので相対位置で区切る
            if (((n - s.burstStart) & 3) == 3)
            {
                zr += cr;
                zi += ci;
                amp = max(amp, 0.5 * sqrt(cr * cr + ci * ci));
                cr = 0.0;
                ci = 0.0;
            }
        }
        lines[l].zr = zr;
        lines[l].zi = zi;
        lines[l].amp = amp;
        mu = atan2(zi, zr) - FRAC_PI_2; // バースト相関の位相 μ
    }
    gA[t] = mu;
    gB[t] = amp;
    GroupMemoryBarrierWithGroupSync();

    // --- PAL アイデント: 隣り合うラインのバースト位相差の多数決で PAL スイッチの向きを決める ---
    float s0 = 1.0;
    if (s.pal)
    {
        float vote = 0.0;
        if (active && t >= 1 && gB[t - 1] >= 3.0 && amp >= 3.0)
        {
            float d = wrapAngle(mu - gA[t - 1] - step);
            if (abs(d) > FRAC_PI_4 && abs(d) < 3.0 * FRAC_PI_4)
            {
                float sv = -sign(d);
                vote = ((t & 1) == 0) ? sv : -sv;
            }
        }
        gC[t] = vote;
        GroupMemoryBarrierWithGroupSync();
        for (int stride = 256; stride > 0; stride >>= 1)
        {
            if (t < stride && t + stride < SERIAL_THREADS)
                gC[t] += gC[t + stride];
            GroupMemoryBarrierWithGroupSync();
        }
        s0 = gC[0] < 0.0 ? -1.0 : 1.0;
    }
    float sv = !s.pal ? 1.0 : (((t & 1) == 0) ? s0 : -s0);
    float measured = s.pal ? mu + FRAC_PI_4 * sv : mu;

    // --- 副搬送波 PLL ---
    float k = clamp(params().burstPll, 0.0, 1.0);
    // VCXO が 1 ラインのあいだに余分に進む位相 (1 次ループなので定常位相誤差として残る)
    float drift = TAU * params().vcxoOffsetHz * float(s.lineSamples) / s.fs;
    if (k < 1.0)
    {
        gC[t] = measured;
        GroupMemoryBarrierWithGroupSync();
        if (t == 0)
        {
            float phase = gC[0];
            for (int l = 1; l < count; l++)
            {
                float pred = wrapAngle(phase + step + drift);
                phase = gB[l] >= 3.0 ? pred + k * wrapAngle(gC[l] - pred) : pred;
                gC[l] = phase;
            }
        }
        GroupMemoryBarrierWithGroupSync();
        measured = gC[t];
    }

    if (active)
    {
        float gain = params().saturation / s.ire.span();
        if (params().acc != 0)
            gain *= min(s.ire.burst / max(amp, 1e-3), 8.0);
        if (params().colorKiller != 0 && amp < 3.0)
            gain = 0.0;
        lines[t].phase = measured;
        lines[t].sv = sv;
        lines[t].gain = gain;
    }
}

/// Y/C 分離 (ノッチ / ラインくし形) と同期検波。スレッド = (ライン, DEMOD_CHUNK サンプルのチャンク)
[shader("compute")]
[numthreads(64, 1, 1)]
void vhsSeparateDemod(uint3 tid : SV_DispatchThreadID)
{
    Standard s = currentStandard();
    int chunks = ceilDiv(s.activeWidth, DEMOD_CHUNK);
    int l = int(tid.x) / chunks;
    int chunk = int(tid.x) % chunks;
    if (l >= s.fieldLines)
        return;

    int lag = combLag(s);
    bool useComb = params().ycMode == 1 && l >= lag;
    var nf = Cascade<1>.single(Biquad.notchAt(s.fsc, 1.0, s.fs));
    var bp = Cascade<1>.single(chromaBandpass(s));
    var aLp = Cascade<1>.butterLowpassAt(params().chromaBwMhz * 1e6, s.fs);
    var bLp = aLp;
    // 輝度はノッチ (またはくし形の色の引き算)、色はバンドパス → 検波ローパスの縦続
    int warmup = max(nf.decaySamples(WARMUP_EPS), bp.decaySamples(WARMUP_EPS) + aLp.decaySamples(WARMUP_EPS));
    // 輝度遅延線: 色の経路 (バンドパス + 検波ローパス) の遅れから輝度の経路の遅れを引いたぶん、色を前へ出す
    float lumaDelay = useComb ? 0.0 : nf.groupDelay(1e3, s.fs);
    int lead = int(max(round(bp.groupDelay(s.fsc, s.fs) + aLp.groupDelay(1e3, s.fs) - lumaDelay), 0.0));
    int activeEnd = s.activeEnd();
    int n0 = s.activeStart + chunk * DEMOD_CHUNK;
    int n1 = min(n0 + DEMOD_CHUNK, activeEnd);
    int start = max(0, n0 - warmup);

    LineShift cur = LineShift(l, s);
    LineShift prev = LineShift(useComb ? l - lag : l, s);
    // NTSC は I/Q 軸 (U/V から 33°) で検波する。PAL は U/V 軸のまま
    float hue = params().hueDeg * PI / 180.0 + (s.pal ? 0.0 : 33.0 * PI / 180.0);
    float psi = lines[l].phase + hue;
    float4 cosT;
    float4 sinT;
    [ForceUnroll]
    for (int k = 0; k < 4; k++)
    {
        float ang = float(k) * FRAC_PI_2 + psi;
        cosT[k] = cos(ang);
        sinT[k] = sin(ang);
    }

    int d = l * s.activeWidth - s.activeStart;
    float a = 0.0;
    float b = 0.0;
    for (int n = start; n < n1 + lead; n++)
    {
        float x = cur.at(min(n, s.lineSamples - 1));
        float y;
        float c;
        if (useComb)
        {
            c = bp.process(0.5 * (x - prev.at(n)));
            y = x - c;
        }
        else
        {
            y = nf.process(x);
            c = bp.process(x);
        }
        // ラインの終わりを越えたぶんは最後の値を保つ
        if (n < s.lineSamples)
        {
            a = aLp.process(2.0 * c * cosT[n & 3]);
            b = bLp.process(2.0 * c * sinT[n & 3]);
        }
        if (n >= n0 && n < n1)
            luma[d + n] = y;
        if (n - lead >= n0)
            detected[d + n - lead] = float2(a, b);
    }
}

/// 輝度・色差 → RGB。このフィールドの行 (2·l + parity) に書く
[shader("compute")]
[numthreads(8, 8, 1)]
void vhsResolve(uint3 tid : SV_DispatchThreadID)
{
    Standard s = currentStandard();
    int x = int(tid.x);
    int l = int(tid.y);
    if (x >= s.activeWidth || l >= s.fieldLines)
        return;

    Levels lv = s.ire;
    LineInfo info = lines[l];
    float2 det = detected[l * s.activeWidth + x];
    float y = luma[l * s.activeWidth + x] - lv.black;
    float u;
    float v;
    if (s.pal)
    {
        v = det.x * info.sv * info.gain;
        u = det.y * info.gain;
        if (params().palDelayLine != 0 && l > 0)
        {
            // 1H 遅延線で前のラインと色差を平均する
            LineInfo pi = lines[l - 1];
            float2 pd = detected[(l - 1) * s.activeWidth + x];
            v = 0.5 * (v + pd.x * pi.sv * pi.gain);
            u = 0.5 * (u + pd.y * pi.gain);
        }
    }
    else
    {
        u = det.x * info.gain; // I
        v = det.y * info.gain; // Q
    }

    float yy = y / lv.span() * params().contrast + params().brightness;
    float3 rgb;
    if (s.pal)
        rgb = float3(yy + 1.140 * v, yy - 0.395 * u - 0.581 * v, yy + 2.032 * u);
    else
        rgb = float3(yy + 0.956 * u + 0.619 * v, yy - 0.272 * u - 0.647 * v, yy - 1.106 * u + 1.703 * v);
    rgb = saturate(rgb);
    if (params().outputLinear != 0)
        rgb = pow(rgb, float3(2.2));
    dstImage.Store(uint2(x, 2 * l + int(params().parity)), float4(rgb, 1.0));
}
