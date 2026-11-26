// テレビ方式のタイミングと信号レベル (crates/vhs-core/src/timing.rs の移植)。
//
// 4fsc でサンプリングした 1 次元のコンポジット信号を IRE 単位で扱う。
// 値は Rust 側の NTSC / PAL 定数と同じにしておくこと。
module timing;

public static const float PI = 3.14159265358979323846;
public static const float FRAC_PI_2 = PI * 0.5;
public static const float FRAC_PI_4 = PI * 0.25;
public static const float TAU = PI * 2.0;
public static const float FRAC_1_SQRT_2 = 0.70710678118654752440;

/// 4fsc サンプリングでの副搬送波の 4 象限位相
public float cos4(int k)
{
    return k == 0 ? 1.0 : (k == 2 ? -1.0 : 0.0);
}

public float sin4(int k)
{
    return k == 1 ? 1.0 : (k == 3 ? -1.0 : 0.0);
}

public struct Levels
{
    public float sync;
    public float blank;
    public float black;
    public float white;
    /// バースト振幅 (片側)
    public float burst;

    public float span() { return white - black; }
}

public struct Standard
{
    public bool pal;
    public float fsc;
    public float fs;
    public int lineSamples;
    public int syncEnd;
    public int burstStart;
    public int burstEnd;
    public int activeStart;
    public int activeWidth;
    public int fieldLines;
    public Levels ire;

    public int activeEnd() { return activeStart + activeWidth; }

    /// ラインごとの副搬送波位相オフセット (90° 単位)
    public int linePhaseBase(int fieldLine, uint frame, uint parity)
    {
        if (pal)
            return (3 * fieldLine + 3 * int(frame & 3) + 3 * int(parity)) & 3;
        return (2 * fieldLine + 2 * int(frame & 1) + 3 * int(parity)) & 3;
    }

    /// 1 ライン進んだときの副搬送波位相の進み (90° 単位)
    public int linePhaseStep() { return lineSamples & 3; }

    /// PAL スイッチ (V の符号)。NTSC は常に +1
    public float vSwitch(int fieldLine, uint frame, uint parity)
    {
        if (!pal)
            return 1.0;
        return ((fieldLine + int(frame & 1) + int(parity)) & 1) == 0 ? 1.0 : -1.0;
    }
}

public Standard ntscStandard()
{
    Standard s;
    s.pal = false;
    s.fsc = 315.0e6 / 88.0;
    s.fs = 4.0 * (315.0e6 / 88.0);
    s.lineSamples = 910;
    s.syncEnd = 67;
    s.burstStart = 76;
    s.burstEnd = 112;
    s.activeStart = 135;
    s.activeWidth = 754;
    s.fieldLines = 240;
    s.ire.sync = -40.0;
    s.ire.blank = 0.0;
    s.ire.black = 7.5;
    s.ire.white = 100.0;
    s.ire.burst = 20.0;
    return s;
}

public Standard palStandard()
{
    Standard s;
    s.pal = true;
    s.fsc = 4433618.75;
    s.fs = 4.0 * 4433618.75;
    s.lineSamples = 1135;
    s.syncEnd = 83;
    s.burstStart = 100;
    s.burstEnd = 140;
    s.activeStart = 186;
    s.activeWidth = 922;
    s.fieldLines = 288;
    s.ire.sync = -300.0 / 7.0;
    s.ire.blank = 0.0;
    s.ire.black = 0.0;
    s.ire.white = 100.0;
    s.ire.burst = 150.0 / 7.0;
    return s;
}

public Standard standardFor(uint kind)
{
    return kind == 1 ? palStandard() : ntscStandard();
}
