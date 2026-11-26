// 撮像管カメラの GPU 版。crates/vhs-core/src/camera.rs の移植。
//
// 入力 (シーンリニアな float の光、または表示用の値) を有効映像の大きさ (activeWidth × 2·fieldLines) で受け、
// RGB 3 管式カメラの出力 (表示用の値。スーパーホワイトは 1 を超えて残る) を camOutput に書く。
// camOutput はそのまま composite.slang の srcImage に渡せる (inputLinear = 0、同じ大きさなので縮小しない)。
//
// ディスパッチの順:
//   入力 1 枚につき 1 回: camOptics → camRowSum → camMean → camBlurH → camBlurV
//   フィールドごと:       camTarget → camProcess
//
// - 光学系 (線形化・露出・混色・フレア・レジストレーション・ぼけ) はフィールドに依らないので入力ごとに 1 回。
// - ターゲット (蓄積・残像・焼き付き・ヒステリシス・ビーム読み出し) は画素ごとに独立なので 1 画素 = 1 スレッド。
//   状態 (電荷・トラップ・ヒステリシス・焼き付き・読み出した信号) は camState にフィールドをまたいで持つ。
// - プロセス回路 (輪郭補償・雑音・ニー・ガンマ) はこのフィールドのラインだけ書く。
//
// 管ごとの個体差と感度・暗電流マップ (しみ・白きずを含む) は、CPU 版と同じ値をホストが作って渡す
// (vhs_core::camera::TubeMaps)。シードから 64 ビットの乱数で作るので GPU では作らない。
// プリアンプ雑音だけは (シード, フィールド, ライン, 管, 画素) をキーにしたハッシュで作るため、CPU 版とは統計的にしか一致しない。
module camera;

import resample;

/// ホストから渡すパラメータ。4 バイトのスカラーだけ、144 バイト (Rust 側の CameraGpuParams と同じ並び)。
/// 管に依らない値は CPU 版と同じ式でホストが前もって計算しておく
public struct CameraGpuParams
{
    // --- 大きさ・フィールド ---
    public uint width;        // 有効映像の幅 (= カメラのターゲットの画素数)
    public uint height;       // フレームのライン数
    public uint srcWidth;
    public uint srcHeight;
    public uint inputLinear;  // 1: 入力はシーンリニア、0: 表示用の値 (2.2 乗で線形化)
    public uint parity;       // このフィールドが読み出すライン (y & 1)
    public uint prime;        // 1: フィールドが連続していないので定常状態から始める
    public uint fieldIndex;   // 通算フィールド番号 (下位 32 ビット、雑音のキー)
    public uint seedLo;
    public uint seedHi;
    public uint act;          // ACT (アンチコメットテール)
    public uint pad0;
    // --- 光学系 ---
    public float gain;        // 2^exposure_ev
    public float crosstalk;   // 0..0.33
    public float flare;       // 画面平均にかける係数 (赤管は 2 倍)
    public float sigma;       // ぼけ [画素]
    // --- ターゲット ---
    public float kDecay;      // 光導電性残像の減衰
    public float kRise;       // 光導電性残像の立ち上がり
    public float actLevel;    // ACT が放電を始める電荷 (読み出し単位)
    public float bias;        // バイアス光
    public float burnRecovery;// 焼き付きの回復 [/フィールド]
    public float hysteresis;  // ヒステリシスの幅
    public float hRelax;      // ヒステリシスが解ける係数
    public float ampCp;       // 真空管プリアンプ (正側の飽和)
    public float ampCn;       // 負側
    public float ampF0;       // 正規化 (黒)
    public float ampScale;    // 正規化 (白)
    // --- プロセス回路 ---
    public float procGain;    // 劣化で落ちた感度を持ち上げる利得
    public float noise;       // プリアンプ雑音 (rms、白レベル比)
    public float aperture;
    public float knee;
    public float kneeSlope;
    public float whiteClip;
    public float cameraGamma;
    public float pad1;
    public float pad2;
}

/// 撮像管 1 本ぶんの値 (vhs_core::camera::Tube から管ごとに決まる値をホストが計算したもの)。48 バイト
public struct TubeGpu
{
    public float gamma;
    public float dx;          // レジストレーションのずれ [画素]
    public float dy;
    public float scale;       // 像の倍率
    public float photoLag;    // 光導電性残像の割合
    public float capLag;      // 容量性残像の K
    public float beam;        // 1 回で読み出せる電荷の上限
    public float qsat;        // ターゲットの電荷の上限
    public float black;       // オートブラックが差し引く黒レベル
    public float burnRate;    // 焼き付きの進む速さ [/フィールド]
    public float pad0;
    public float pad1;
}

/// ターゲットの画素ごとの状態。xyz = R/G/B 管
public struct CamPixel
{
    public float4 q;     // 蓄積電荷
    public float4 trap;  // 光導電性残像のトラップ
    public float4 hyst;  // ヒステリシス
    public float4 burn;  // 焼き付き (感度低下率)
    public float4 sig;   // 読み出した信号 (プリアンプ後)
}

/// 真空管プリアンプの動作点 (白レベル比)
static const float AMP_BIAS = 0.35;

/// [0] だけ使う
StructuredBuffer<CameraGpuParams> camParams;
/// [0..3] = R/G/B 管
StructuredBuffer<TubeGpu> camTubes;
/// カメラに入る光 (srcWidth × srcHeight)
Texture2D<float4> camSource;
/// カメラの出力 (width × height、表示用の値)。このフィールドのラインだけ書く
[format("rgba32f")]
WTexture2D<float4> camOutput;
/// [2i] = 感度マップ (シェーディング・しみ)、[2i+1] = 暗電流マップ (傾き・白きず)。xyz = R/G/B 管。
/// WebGPU の既定の上限 (ストレージバッファ 8 本) に収めるため 1 本にまとめている
StructuredBuffer<float4> camMaps;
/// 線形化・露出・混色をかけた光
RWStructuredBuffer<float4> camLin;
/// 横方向にぼかした光
RWStructuredBuffer<float4> camTmp;
/// ターゲットに当たる光 (フレア・レジストレーション・ぼけの後)
RWStructuredBuffer<float4> camFlux;
RWStructuredBuffer<CamPixel> camState;
/// [y] = camLin の行ごとの和、[height] = 画面平均
RWStructuredBuffer<float4> camRows;

CameraGpuParams cp() { return camParams[0]; }

/// pow の底が 0 のときを明示的に 0 にする (Rust の powf と同じ結果にする)
float powSafe(float v, float e) { return v > 0.0 ? pow(v, e) : 0.0; }

// ---------------------------------------------------------------------------
// 光学系 (入力 1 枚につき 1 回)
// ---------------------------------------------------------------------------

float3 loadLight(int x, int y, bool linear)
{
    float3 c = camSource.Load(int3(x, y, 0)).rgb;
    // 表示用の値は 8 bit 入力と同じく表示ガンマ 2.2 として線形化する (白を超える光は表せない)
    return linear ? sanitize(c) : pow(clamp(c, float3(0.0), float3(1.0)), float3(2.2));
}

/// 入力を有効映像の大きさに合わせて読み (LinearFrame::resized と同じ三角フィルタ)、露出と混色をかける
[shader("compute")]
[numthreads(8, 8, 1)]
void camOptics(uint3 tid : SV_DispatchThreadID)
{
    CameraGpuParams p = cp();
    int w = int(p.width);
    int h = int(p.height);
    int x = int(tid.x);
    int y = int(tid.y);
    if (x >= w || y >= h)
        return;
    int2 size = int2(int(p.srcWidth), int(p.srcHeight));
    bool linear = p.inputLinear != 0;
    float3 rgb;
    if (size.x == w && size.y == h)
    {
        rgb = loadLight(x, y, linear);
    }
    else
    {
        ResampleAxis ax = ResampleAxis(x, size.x, w);
        ResampleAxis ay = ResampleAxis(y, size.y, h);
        float wsumX = 0.0;
        for (int j = ax.lo; j < ax.hi; j++)
            wsumX += ax.weight(j);
        float wsumY = 0.0;
        float3 acc = float3(0.0);
        for (int i = ay.lo; i < ay.hi; i++)
        {
            float wy = ay.weight(i);
            if (wy <= 0.0)
                continue;
            wsumY += wy;
            float3 row = float3(0.0);
            for (int j = ax.lo; j < ax.hi; j++)
                row += loadLight(j, i, linear) * ax.weight(j);
            acc += row * wy;
        }
        rgb = acc / max(wsumX * wsumY, 1e-20);
    }
    rgb *= p.gain;
    // ダイクロイックプリズムの分光特性の裾で隣の色が漏れ込む
    float k = p.crosstalk;
    float3 mixed = float3(
        rgb.r * (1.0 - k) + rgb.g * k,
        rgb.g * (1.0 - 2.0 * k) + (rgb.r + rgb.b) * k,
        rgb.b * (1.0 - k) + rgb.g * k);
    camLin[y * w + x] = float4(mixed, 0.0);
}

/// 行ごとの和 (フレア用)。CPU 版と同じく行の中は先頭から順に足す
[shader("compute")]
[numthreads(64, 1, 1)]
void camRowSum(uint3 tid : SV_DispatchThreadID)
{
    CameraGpuParams p = cp();
    int w = int(p.width);
    int y = int(tid.x);
    if (y >= int(p.height))
        return;
    float4 acc = float4(0.0);
    for (int x = 0; x < w; x++)
        acc += camLin[y * w + x];
    camRows[y] = acc;
}

/// 画面平均。加算順を CPU 版と同じに固定する (行の和を上から順に足す)
[shader("compute")]
[numthreads(1, 1, 1)]
void camMean(uint3 tid : SV_DispatchThreadID)
{
    CameraGpuParams p = cp();
    int h = int(p.height);
    float4 acc = float4(0.0);
    for (int y = 0; y < h; y++)
        acc += camRows[y];
    camRows[h] = acc / float(p.width * p.height);
}

int blurRadius(float sigma) { return sigma < 0.05 ? 0 : int(ceil(sigma * 3.0)); }

/// 正規化したガウス窓の重み (i = -r..r)
float blurWeight(int i, float sigma, float norm)
{
    return sigma < 0.05 ? 1.0 : exp(-float(i * i) / (2.0 * sigma * sigma)) / norm;
}

float blurNorm(float sigma)
{
    int r = blurRadius(sigma);
    float sum = 0.0;
    for (int i = -r; i <= r; i++)
        sum += exp(-float(i * i) / (2.0 * sigma * sigma));
    return sigma < 0.05 ? 1.0 : sum;
}

/// レジストレーションのずれ (管ごとの平行移動と倍率) をかけた光 + フレア
float shiftedLight(int c, int x, int y, int w, int h, float flare)
{
    TubeGpu t = camTubes[c];
    bool shifted = abs(t.dx) > 1e-3 || abs(t.dy) > 1e-3 || abs(t.scale - 1.0) > 1e-6;
    if (!shifted)
        return camLin[y * w + x][c] + flare;
    float cx = float(w) * 0.5;
    float cy = float(h) * 0.5;
    float sy = clamp(cy + (float(y) + 0.5 - cy) * t.scale - t.dy - 0.5, 0.0, float(h - 1));
    int y0 = int(sy);
    int y1 = min(y0 + 1, h - 1);
    float fy = sy - float(y0);
    float sx = clamp(cx + (float(x) + 0.5 - cx) * t.scale - t.dx - 0.5, 0.0, float(w - 1));
    int x0 = int(sx);
    int x1 = min(x0 + 1, w - 1);
    float fx = sx - float(x0);
    float a = camLin[y0 * w + x0][c];
    float b = camLin[y0 * w + x1][c];
    float d = camLin[y1 * w + x0][c];
    float e = camLin[y1 * w + x1][c];
    float top = a + (b - a) * fx;
    float bot = d + (e - d) * fx;
    return top + (bot - top) * fy + flare;
}

/// フレア・レジストレーション・横方向のぼけ
[shader("compute")]
[numthreads(8, 8, 1)]
void camBlurH(uint3 tid : SV_DispatchThreadID)
{
    CameraGpuParams p = cp();
    int w = int(p.width);
    int h = int(p.height);
    int x = int(tid.x);
    int y = int(tid.y);
    if (x >= w || y >= h)
        return;
    float4 mean = camRows[h];
    int r = blurRadius(p.sigma);
    float norm = blurNorm(p.sigma);
    float4 outv = float4(0.0);
    [unroll]
    for (int c = 0; c < 3; c++)
    {
        // フレア: 画面全体の光がかぶる (赤管は特に多い)
        float flare = p.flare * (c == 0 ? 2.0 : 1.0) * mean[c];
        float acc = 0.0;
        for (int i = -r; i <= r; i++)
            acc += shiftedLight(c, clamp(x + i, 0, w - 1), y, w, h, flare) * blurWeight(i, p.sigma, norm);
        outv[c] = acc;
    }
    camTmp[y * w + x] = outv;
}

/// 縦方向のぼけ
[shader("compute")]
[numthreads(8, 8, 1)]
void camBlurV(uint3 tid : SV_DispatchThreadID)
{
    CameraGpuParams p = cp();
    int w = int(p.width);
    int h = int(p.height);
    int x = int(tid.x);
    int y = int(tid.y);
    if (x >= w || y >= h)
        return;
    int r = blurRadius(p.sigma);
    float norm = blurNorm(p.sigma);
    float4 acc = float4(0.0);
    for (int i = -r; i <= r; i++)
        acc += camTmp[clamp(y + i, 0, h - 1) * w + x] * blurWeight(i, p.sigma, norm);
    camFlux[y * w + x] = acc;
}

// ---------------------------------------------------------------------------
// 光導電ターゲットと読み出し (フィールドごと)
// ---------------------------------------------------------------------------

/// 真空管プリアンプの正負非対称な飽和特性 (黒と白が動かないよう正規化してある)
float ampApply(CameraGpuParams p, float s)
{
    float u = s - AMP_BIAS;
    float c = u > 0.0 ? p.ampCp : p.ampCn;
    float raw = c < 1e-4 ? u : tanh(c * u) / c;
    return (raw - p.ampF0) * p.ampScale;
}

/// ターゲットへの蓄積と、このフィールドの走査線の読み出し。全ラインに光が当たり、読むのはこのフィールドのラインだけ
[shader("compute")]
[numthreads(8, 8, 1)]
void camTarget(uint3 tid : SV_DispatchThreadID)
{
    CameraGpuParams p = cp();
    int w = int(p.width);
    int x = int(tid.x);
    int y = int(tid.y);
    if (x >= w || y >= int(p.height))
        return;
    int idx = y * w + x;
    bool readLine = (uint(y) & 1u) == p.parity;
    bool prime = p.prime != 0;
    float4 flux = camFlux[idx];
    float4 sens = camMaps[2 * idx];
    float4 dark = camMaps[2 * idx + 1];
    CamPixel st = camState[idx];

    [unroll]
    for (int c = 0; c < 3; c++)
    {
        TubeGpu t = camTubes[c];
        float f = max(flux[c], 0.0);
        float i = sens[c] * (1.0 - st.burn[c]) * powSafe(f, t.gamma) + dark[c] + p.bias;
        float kc = t.capLag;
        if (prime)
        {
            st.trap[c] = i;
            st.burn[c] = 0.0;
            // 定常状態: 読み出し直前の電荷 Q は Q²/(Q+K) = 2i を満たす
            float qeq = i + sqrt(i * i + 2.0 * i * kc);
            float residual = max(qeq - 2.0 * i, 0.0);
            st.q[c] = residual + (readLine ? i : 0.0);
        }
        // 光導電性残像 (トラップ)
        float trap = st.trap[c];
        float k = i > trap ? p.kRise : p.kDecay;
        trap = trap + (i - trap) * k;
        st.trap[c] = trap;
        float ie = i * (1.0 - t.photoLag) + trap * t.photoLag;

        // 焼き付き: 強い光で感度が落ち、ゆっくり戻る
        float b = st.burn[c] + t.burnRate * max(f - 0.6, 0.0) - p.burnRecovery * st.burn[c];
        st.burn[c] = clamp(b, 0.0, 0.95);

        float q = min(st.q[c] + ie, t.qsat);
        if (readLine)
        {
            // 容量性残像 (低照度ほど読み残す) とビーム電流制限 (コメットテール)
            float rd = min(kc > 0.0 ? q * q / (q + kc) : q, t.beam);
            q -= rd;
            if (p.act != 0)
                q = min(q, p.actLevel);
            float s = rd * 0.5 - t.black;
            // ヒステリシス (遊び演算子 + 緩和)
            float hw = p.hysteresis;
            float hv = prime ? s : clamp(st.hyst[c], s - hw, s + hw);
            hv = hv + (s - hv) * p.hRelax;
            st.hyst[c] = hv;
            st.sig[c] = ampApply(p, hw > 0.0 ? hv : s);
        }
        st.q[c] = q;
    }
    camState[idx] = st;
}

// ---------------------------------------------------------------------------
// プロセス回路 (フィールドごと、このフィールドのラインだけ)
// ---------------------------------------------------------------------------

uint hash32(uint v)
{
    v ^= v >> 16;
    v *= 0x7feb352du;
    v ^= v >> 15;
    v *= 0x846ca68bu;
    v ^= v >> 16;
    return v;
}

/// 平均 0・分散 1 の近似正規乱数 (vhs-core の Rng::normal と同じく 16 ビットの一様乱数 4 個の和)
float gaussian(uint key)
{
    uint a = hash32(key);
    uint b = hash32(a ^ 0x9e3779b9u);
    float sum = float(a & 0xffffu) + float(a >> 16) + float(b & 0xffffu) + float(b >> 16);
    return (sum - 2.0 * 65535.0) * (1.0 / (65536.0 * 0.5773503));
}

float sigAt(int x, int y, int w, int c) { return camState[y * w + x].sig[c]; }

/// スレッド = (画素, このフィールドのライン)
[shader("compute")]
[numthreads(8, 8, 1)]
void camProcess(uint3 tid : SV_DispatchThreadID)
{
    CameraGpuParams p = cp();
    int w = int(p.width);
    int h = int(p.height);
    int x = int(tid.x);
    int y = int(tid.y) * 2 + int(p.parity);
    if (x >= w || y >= h)
        return;
    // 同じフィールドの上下のライン (1H 遅延線による垂直輪郭補償)
    int yu = y >= 2 ? y - 2 : y;
    int yd = y + 2 < h ? y + 2 : y;
    int xl = max(x - 1, 0);
    int xr = min(x + 1, w - 1);
    uint seedKey = hash32(p.seedLo ^ hash32(p.seedHi + 0x632be5abu));
    float4 outv = float4(0.0, 0.0, 0.0, 1.0);
    [unroll]
    for (int c = 0; c < 3; c++)
    {
        float v0 = sigAt(x, y, w, c);
        float around = 0.25 * (sigAt(xl, y, w, c) + sigAt(xr, y, w, c) + sigAt(x, yu, w, c) + sigAt(x, yd, w, c));
        float v = v0 + p.aperture * (v0 - around);
        // 前置増幅器の雑音: 入力容量のため高域ほど大きい (三角雑音)。左隣の画素の乱数との差を足す
        if (p.noise > 0.0)
        {
            uint lineKey = hash32(seedKey ^ hash32(p.fieldIndex * 0x9e3779b9u + uint(y) * 3u + uint(c)));
            float n = gaussian(lineKey + uint(x) * 0x85ebca6bu);
            float prev = x > 0 ? gaussian(lineKey + uint(x - 1) * 0x85ebca6bu) : 0.0;
            v += p.noise * (0.35 * n + 0.75 * (n - prev));
        }
        v *= p.procGain;
        if (v > p.knee)
            v = p.knee + (v - p.knee) * p.kneeSlope;
        // スーパーホワイト (白 = 1.0 を超える分) はそのまま信号に残す
        outv[c] = powSafe(max(min(v, p.whiteClip), 0.0), p.cameraGamma);
    }
    camOutput.Store(uint2(x, y), outv);
}
