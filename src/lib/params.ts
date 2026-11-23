// vhs-core の Params と対応する型と、GUI 用のパラメータ定義。
// Rust 側 (crates/vhs-core/src/params.rs) を変更したらここも合わせる。

export type Params = {
  encoder: { luma_bw_mhz: number; i_bw_mhz: number; q_bw_mhz: number; chroma_gain: number };
  tape: {
    enabled: boolean;
    luma_bw_mhz: number;
    emphasis_db: number;
    emphasis_mhz: number;
    white_clip_ire: number;
    bearding_threshold_ire: number;
    bearding_strength: number;
    luma_noise_ire: number;
    sharpness_db: number;
    chroma_bw_khz: number;
    chroma_noise_ire: number;
    chroma_phase_noise_deg: number;
    chroma_delay_samples: number;
    magnetic_model: "linear" | "tanh";
    saturation_ire: number;
    dropout_rate: number;
    dropout_length: number;
    dropout_compensation: boolean;
    crease_rate: number;
    crease_fields: number;
    head_clog_rate: number;
  };
  transport: {
    jitter_samples: number;
    jitter_correlation: number;
    wow_samples: number;
    wow_hz: number;
    head_switch_lines: number;
    head_switch_offset: number;
    head_switch_noise: number;
    skew_samples: number;
    tracking_error: number;
    tracking_speed: number;
    vertical_jitter: number;
  };
  rf: {
    enabled: boolean;
    snow_ire: number;
    ghost_delay_us: number;
    ghost_gain: number;
    interference_ire: number;
    interference_mhz: number;
    sound_beat: number;
  };
  decoder: {
    yc_separation: "notch" | "comb";
    chroma_bw_mhz: number;
    acc: boolean;
    color_killer: boolean;
    h_pll: number;
    contrast: number;
    brightness: number;
    saturation: number;
    hue_deg: number;
  };
  capture: {
    enabled: boolean;
    levels: "correct" | "washed_out" | "crushed";
    chroma_subsampling: "444" | "422" | "420" | "411";
    codec: boolean;
    bitrate_kbps: number;
    gop: number;
    motion_search: number;
    rate_control: number;
    frame_drop_rate: number;
    adc_bits: number;
  };
  audio: {
    enabled: boolean;
    mode: "linear" | "hifi";
    low_cut_hz: number;
    high_cut_hz: number;
    hiss_db: number;
    drive_db: number;
    wow_pct: number;
    flutter_pct: number;
    dropout_rate: number;
    hifi_buzz: number;
    hum_db: number;
    hum_hz: number;
    agc: number;
    sample_rate_hz: number;
    bits: number;
    mono: boolean;
    codec_kbps: number;
  };
  output: { deinterlace: "weave" | "bob" };
  osd: {
    enabled: boolean;
    mode: "composite" | "tape";
    markup: string;
    scale: number;
    opacity: number;
    start_datetime: string;
  };
  seed: number;
};

export type Control =
  | { kind: "range"; path: string; label: string; min: number; max: number; step: number; unit?: string }
  | { kind: "bool"; path: string; label: string }
  | { kind: "select"; path: string; label: string; options: { value: string; label: string }[] }
  | { kind: "text"; path: string; label: string; placeholder?: string }
  | { kind: "markup"; path: string; label: string };

export type Group = { title: string; note?: string; controls: Control[] };

const r = (path: string, label: string, min: number, max: number, step: number, unit?: string): Control => ({
  kind: "range", path, label, min, max, step, unit,
});
const b = (path: string, label: string): Control => ({ kind: "bool", path, label });

export const groups: Group[] = [
  {
    title: "OSD (画面表示)",
    note:
      "タグ: <color=色> <bg=色> <edge=色> <shadow=色> <size=1〜8> <blink> は </名前> で閉じる。" +
      "<pos=X,Y> (754×480 座標) と <align=left|center|right> は単独。" +
      "色は #rgb / #rrggbb / #rrggbbaa / white, black, red, green, blue, yellow, cyan, magenta, gray, orange, none。" +
      "置換: {counter} {date} {time} {Y} {M} {D} {h} {m} {s} {MON} {h12} {ampm} {f}。" +
      "記号: ▶ ◀ ■ ● ▲ ▼ ⏸ ❚ █ °。\\< \\{ で記号そのもの。",
    controls: [
      b("osd.enabled", "OSD を表示"),
      {
        kind: "select", path: "osd.mode", label: "合成先",
        options: [
          { value: "composite", label: "デッキ出力に重畳 (コンポジット)" },
          { value: "tape", label: "テープに焼き込み (録画済み)" },
        ],
      },
      { kind: "markup", path: "osd.markup", label: "表示内容" },
      r("osd.scale", "文字サイズ", 1, 8, 1),
      r("osd.opacity", "不透明度", 0, 1, 0.01),
      { kind: "text", path: "osd.start_datetime", label: "開始日時", placeholder: "1995-08-12 14:30:00" },
    ],
  },
  {
    title: "テープ: 輝度 (FM 記録)",
    controls: [
      b("tape.enabled", "テープを通す"),
      r("tape.luma_bw_mhz", "輝度帯域", 0.5, 5, 0.05, "MHz"),
      r("tape.luma_noise_ire", "FM ノイズ", 0, 20, 0.1, "IRE"),
      r("tape.emphasis_db", "エンファシス", 0, 20, 0.5, "dB"),
      r("tape.emphasis_mhz", "エンファシス周波数", 0.1, 3, 0.05, "MHz"),
      r("tape.white_clip_ire", "ホワイトクリップ", 100, 250, 1, "IRE"),
      r("tape.bearding_threshold_ire", "ベアディング閾値", 100, 250, 1, "IRE"),
      r("tape.bearding_strength", "ベアディング強度", 0, 3, 0.05),
      r("tape.sharpness_db", "再生ピーキング", -12, 12, 0.5, "dB"),
    ],
  },
  {
    title: "テープ: 色 (低域変換記録)",
    controls: [
      r("tape.chroma_bw_khz", "クロマ帯域", 100, 1500, 10, "kHz"),
      r("tape.chroma_noise_ire", "クロマノイズ", 0, 20, 0.1, "IRE"),
      r("tape.chroma_phase_noise_deg", "位相ノイズ", 0, 45, 0.5, "°"),
      r("tape.chroma_delay_samples", "Y/C 遅延", -10, 20, 1, "smp"),
      {
        kind: "select", path: "tape.magnetic_model", label: "磁性体モデル",
        options: [{ value: "tanh", label: "tanh 飽和" }, { value: "linear", label: "線形" }],
      },
      r("tape.saturation_ire", "飽和レベル", 5, 150, 1, "IRE"),
    ],
  },
  {
    title: "テープ: ドロップアウト",
    controls: [
      r("tape.dropout_rate", "発生率 (/ライン)", 0, 0.1, 0.0005),
      r("tape.dropout_length", "平均長", 1, 400, 1, "smp"),
      b("tape.dropout_compensation", "ドロップアウト補償 (DOC)"),
    ],
  },
  {
    title: "テープ: 傷み",
    note: "しわは画面を流れる帯状のノイズ、目詰まりは画面全体が一瞬砂嵐になる。",
    controls: [
      r("tape.crease_rate", "しわ (回/分)", 0, 30, 0.5),
      r("tape.crease_fields", "しわの長さ", 2, 120, 1, "field"),
      r("tape.head_clog_rate", "ヘッド目詰まり (回/分)", 0, 30, 0.5),
    ],
  },
  {
    title: "走行系・ヘッド",
    controls: [
      r("transport.jitter_samples", "時間軸ジッタ", 0, 5, 0.05, "smp"),
      r("transport.jitter_correlation", "ジッタ相関", 0, 0.999, 0.001),
      r("transport.wow_samples", "ワウフラッタ", 0, 20, 0.1, "smp"),
      r("transport.wow_hz", "ワウ周波数", 0.1, 30, 0.1, "Hz"),
      r("transport.head_switch_lines", "ヘッド切替ライン数", 0, 30, 1),
      r("transport.head_switch_offset", "ヘッド切替ずれ", -40, 40, 0.5, "smp"),
      r("transport.head_switch_noise", "ヘッド切替ノイズ", 0, 3, 0.05),
      r("transport.skew_samples", "スキュー", -40, 40, 0.5, "smp"),
      r("transport.tracking_error", "トラッキングずれ", 0, 1, 0.01),
      r("transport.tracking_speed", "ノイズバー速度", -10, 10, 0.1, "line/f"),
      r("transport.vertical_jitter", "垂直揺れ", 0, 4, 0.05, "line"),
    ],
  },
  {
    title: "RF 接続 (アンテナ端子)",
    note: "デッキの RF 出力をテレビのチューナで受けたときの劣化。",
    controls: [
      b("rf.enabled", "RF 経由で受像"),
      r("rf.snow_ire", "スノー", 0, 30, 0.1, "IRE"),
      r("rf.ghost_gain", "ゴースト", -0.6, 0.6, 0.01),
      r("rf.ghost_delay_us", "ゴースト遅延", 0.1, 10, 0.05, "µs"),
      r("rf.interference_ire", "ビート妨害", 0, 20, 0.1, "IRE"),
      r("rf.interference_mhz", "ビート周波数", 0.1, 4.5, 0.01, "MHz"),
      r("rf.sound_beat", "音声ビート", 0, 3, 0.01),
    ],
  },
  {
    title: "エンコーダ (送出側)",
    controls: [
      r("encoder.luma_bw_mhz", "輝度帯域", 1, 6, 0.05, "MHz"),
      r("encoder.i_bw_mhz", "I 帯域", 0.2, 3, 0.05, "MHz"),
      r("encoder.q_bw_mhz", "Q 帯域", 0.1, 3, 0.05, "MHz"),
      r("encoder.chroma_gain", "色ゲイン", 0, 3, 0.01),
    ],
  },
  {
    title: "受像機 (デコーダ)",
    controls: [
      {
        kind: "select", path: "decoder.yc_separation", label: "Y/C 分離",
        options: [{ value: "notch", label: "ノッチ (ドットクロール)" }, { value: "comb", label: "1H くし形" }],
      },
      r("decoder.chroma_bw_mhz", "色差帯域", 0.2, 3, 0.05, "MHz"),
      b("decoder.acc", "ACC"),
      b("decoder.color_killer", "カラーキラー"),
      r("decoder.h_pll", "水平 AFC 追従", 0, 1, 0.01),
      r("decoder.contrast", "コントラスト", 0, 2, 0.01),
      r("decoder.brightness", "明るさ", -0.5, 0.5, 0.01),
      r("decoder.saturation", "色の濃さ", 0, 3, 0.01),
      r("decoder.hue_deg", "色相", -180, 180, 1, "°"),
      {
        kind: "select", path: "output.deinterlace", label: "インターレース",
        options: [{ value: "weave", label: "Weave" }, { value: "bob", label: "Bob" }],
      },
    ],
  },
  {
    title: "キャプチャ (デジタル化)",
    note: "受像した映像をキャプチャ機器で取り込み MPEG 系で圧縮したときの見た目。ビットレートは 29.97 fps 換算。",
    controls: [
      b("capture.enabled", "キャプチャを通す"),
      {
        kind: "select", path: "capture.levels", label: "レベル",
        options: [
          { value: "correct", label: "正しい (16-235)" },
          { value: "washed_out", label: "黒浮き (16-235 をフル扱い)" },
          { value: "crushed", label: "黒つぶれ (フルを 16-235 扱い)" },
        ],
      },
      {
        kind: "select", path: "capture.chroma_subsampling", label: "色差間引き",
        options: [
          { value: "420", label: "4:2:0 (MPEG-2 / DVD)" },
          { value: "411", label: "4:1:1 (DV)" },
          { value: "422", label: "4:2:2" },
          { value: "444", label: "4:4:4 (なし)" },
        ],
      },
      r("capture.adc_bits", "ADC ビット数", 3, 8, 1, "bit"),
      b("capture.codec", "DCT 圧縮"),
      r("capture.bitrate_kbps", "ビットレート", 200, 15000, 50, "kbps"),
      r("capture.gop", "GOP 長", 1, 60, 1, "frame"),
      r("capture.motion_search", "動き探索範囲", 0, 32, 1, "px"),
      r("capture.rate_control", "レート制御の追従", 0, 1, 0.01),
      r("capture.frame_drop_rate", "コマ落ち", 0, 0.3, 0.001),
    ],
  },
  {
    title: "音声 (書き出し時)",
    note: "動画の書き出し時に音声トラックへ適用する (プレビューには音声なし)。",
    controls: [
      b("audio.enabled", "音声を加工する"),
      {
        kind: "select", path: "audio.mode", label: "音声トラック",
        options: [
          { value: "linear", label: "リニア (ノーマル, モノラル)" },
          { value: "hifi", label: "Hi-Fi (ステレオ)" },
        ],
      },
      r("audio.low_cut_hz", "低域カット", 20, 400, 1, "Hz"),
      r("audio.high_cut_hz", "高域カット", 2000, 20000, 100, "Hz"),
      r("audio.drive_db", "録音レベル", -12, 18, 0.5, "dB"),
      r("audio.hiss_db", "ヒス", -90, -20, 0.5, "dBFS"),
      r("audio.wow_pct", "ワウ", 0, 2, 0.01, "%"),
      r("audio.flutter_pct", "フラッタ", 0, 1, 0.01, "%"),
      r("audio.dropout_rate", "ドロップアウト", 0, 3, 0.01, "/s"),
      r("audio.hifi_buzz", "Hi-Fi バズ", 0, 2, 0.01),
      r("audio.hum_db", "電源ハム", -90, -20, 0.5, "dBFS"),
      r("audio.hum_hz", "電源周波数", 50, 60, 10, "Hz"),
      r("audio.agc", "AGC", 0, 1, 0.01),
      r("audio.sample_rate_hz", "ADC サンプリング", 8000, 48000, 1000, "Hz"),
      r("audio.bits", "ADC ビット数", 4, 24, 1, "bit"),
      b("audio.mono", "モノラル"),
      r("audio.codec_kbps", "AAC ビットレート", 16, 320, 8, "kbps"),
    ],
  },
];

// ツールチップ用の説明 (path → 説明文)。
export const help: Record<string, string> = {
  // OSD
  "osd.enabled": "デッキやカメラの画面表示 (PLAY、カウンタ、日付など) を重ねる。",
  "osd.mode":
    "デッキ出力に重畳: 再生機が出力に文字を重ねる (くっきり、テープ劣化を受けない)。\n" +
    "テープに焼き込み: 録画時に記録された文字 (カメラの日付など)。テープの劣化を一緒に受ける。",
  "osd.markup": "表示内容のマークアップ。書式はグループ上部の説明を参照。",
  "osd.scale": "既定の文字サイズ。1 = 1 ドットが 2×2 画素。<size=> タグで部分的に変更できる。",
  "osd.opacity": "OSD 全体の不透明度。0 で透明、1 で不透明。",
  "osd.start_datetime": "{date} {time} などの起点となる日時 (YYYY-MM-DD HH:MM:SS)。再生時間に合わせて進む。",

  // テープ: 輝度
  "tape.enabled": "オフにするとテープを通さず、放送をそのまま受像した映像になる。",
  "tape.luma_bw_mhz": "輝度の記録帯域。小さいほど映像がぼやける。VHS SP ≒ 3.0 MHz、3倍モードはさらに狭い。",
  "tape.luma_noise_ire": "FM 復調で生じる輝度ノイズの量 (rms)。ザラザラした粒状ノイズになる。",
  "tape.emphasis_db": "記録時に高域を持ち上げる量。大きいほど輪郭のオーバーシュートやクリップが起きやすい。",
  "tape.emphasis_mhz": "エンファシスが効き始める周波数。",
  "tape.white_clip_ire": "エンファシス後の信号を切り詰めるレベル。低いほど明るい輪郭の白が潰れる。",
  "tape.bearding_threshold_ire": "FM 過変調で白が黒く反転 (ベアディング) し始めるレベル。低いほど起きやすい。",
  "tape.bearding_strength": "ベアディング (明るい輪郭の右側に出る黒い筋) の強さ。",
  "tape.sharpness_db": "再生時の輪郭強調。負で甘く、正でシャープ (リンギングが目立つ)。",

  // テープ: 色
  "tape.chroma_bw_khz": "低域変換された色信号の帯域。小さいほど色がにじむ。VHS は約 500 kHz。",
  "tape.chroma_noise_ire": "色信号の振幅ノイズ (rms)。色ムラ・色の粒状ノイズになる。",
  "tape.chroma_phase_noise_deg": "再生時の色位相の揺らぎ (rms)。ラインごとに色相がぶれる。",
  "tape.chroma_delay_samples": "輝度に対する色の位置ずれ。正で色が右にずれる。",
  "tape.magnetic_model": "磁性体の伝達特性。tanh は強い色が飽和して頭打ちになる。線形は飽和なし。",
  "tape.saturation_ire": "磁気飽和が始まる色の振幅。小さいほど鮮やかな色が早く潰れる (tanh のみ)。",

  // テープ: ドロップアウト
  "tape.dropout_rate": "1 ラインあたりのドロップアウト (磁性体の欠落による白や黒の横筋) の発生確率。",
  "tape.dropout_length": "ドロップアウト 1 回の平均の長さ。",
  "tape.dropout_compensation": "ドロップアウト部分を 1 ライン前の映像で埋める (DOC)。オフだと白い筋がそのまま出る。",

  // テープ: 傷み
  "tape.crease_rate": "テープのしわ・折れ目が現れる頻度。画面を帯状のノイズが流れる。",
  "tape.crease_fields": "しわ 1 回が画面に現れ続ける長さ (フィールド数、60 で約 1 秒)。",
  "tape.head_clog_rate": "ヘッドの目詰まりが起きる頻度。画面全体が一瞬砂嵐に埋もれる。",

  // 走行系
  "transport.jitter_samples": "ラインごとの時間軸の揺れ (rms)。縦線がギザギザに揺れる。",
  "transport.jitter_correlation": "ジッタの隣接ライン間の相関。1 に近いほどゆっくりうねる揺れになる。",
  "transport.wow_samples": "周期的な時間軸の揺れの振幅。画面全体が左右にゆらゆら揺れる。",
  "transport.wow_hz": "ワウフラッタの揺れの周波数。",
  "transport.head_switch_lines": "画面下端でヘッド切替ノイズが及ぶライン数。",
  "transport.head_switch_offset": "ヘッド切替部分の水平方向のずれ。画面下端が横に曲がる。",
  "transport.head_switch_noise": "ヘッド切替部分に乗るノイズの量。",
  "transport.skew_samples": "テープ張力の誤差による画面上部の曲がり (フラッギング)。",
  "transport.tracking_error": "トラッキングのずれ。大きいほどノイズバーが太く濃くなる。",
  "transport.tracking_speed": "ノイズバーが画面を移動する速さ。負で上向き。",
  "transport.vertical_jitter": "画面全体の上下の揺れ (rms)。安物デッキの V ジッタ。",

  // RF
  "rf.enabled": "デッキの RF 出力 (アンテナ端子) をテレビのチューナで受けた状態にする。",
  "rf.snow_ire": "受信ノイズ (スノー) の量。電波が弱いときの砂嵐。",
  "rf.ghost_gain": "反射波によるゴースト (二重像) の強さ。負にすると反転ゴースト。",
  "rf.ghost_delay_us": "ゴーストの遅れ。大きいほど二重像が右に離れる。",
  "rf.interference_ire": "ビート妨害 (斜めの縞模様) の強さ。",
  "rf.interference_mhz": "ビート妨害の周波数。縞の細かさ・角度が変わる。",
  "rf.sound_beat": "音声搬送波が映像に漏れる量。920 kHz ビートや音声に連動する縞が出る。",

  // エンコーダ
  "encoder.luma_bw_mhz": "送出側の輝度帯域。NTSC 放送は 4.2 MHz。",
  "encoder.i_bw_mhz": "I 信号 (橙〜青緑方向の色差) の帯域。NTSC 規格は 1.3 MHz。",
  "encoder.q_bw_mhz": "Q 信号 (紫〜黄緑方向の色差) の帯域。NTSC 規格は 0.6 MHz。",
  "encoder.chroma_gain": "送出時の色信号の振幅倍率。大きいとテープでの色飽和が起きやすい。",

  // デコーダ
  "decoder.yc_separation":
    "輝度と色の分離方式。ノッチは安価な方式でドットクロールや虹色のクロスカラーが出る。" +
    "1H くし形はそれらを抑えるが、縦方向の色の境目がにじむ。",
  "decoder.chroma_bw_mhz": "受像機で復調した後の色差の帯域。小さいほど色がにじむ。",
  "decoder.acc": "カラーバーストの振幅で色の濃さを自動調整する (ACC)。",
  "decoder.color_killer": "カラーバーストが弱いとき色を消して白黒表示にする。",
  "decoder.h_pll": "水平同期 (AFC) の追従の速さ。0 = 追従しない、1 = 即時追従。小さいと時間軸の揺れがそのまま見える。",
  "decoder.contrast": "受像機のコントラスト (映像の振幅倍率)。",
  "decoder.brightness": "受像機の明るさ (黒レベルのオフセット)。",
  "decoder.saturation": "受像機の色の濃さ。",
  "decoder.hue_deg": "受像機の色相 (ティント)。NTSC 特有の色ずれを再現・補正できる。",
  "output.deinterlace": "Weave: 2 フィールドを交互に織り込む (動きに櫛状のずれ)。Bob: 各フィールドのラインを 2 倍に伸ばす。",

  // キャプチャ
  "capture.enabled": "受像した映像をキャプチャ機器でデジタル化・圧縮した状態にする。",
  "capture.levels":
    "ビデオレンジ (16-235) とフルレンジの扱い。誤ると黒が浮いてくすむ (黒浮き) か、" +
    "黒つぶれ・白飛びが起きる。",
  "capture.chroma_subsampling": "色差の間引き方。4:2:0 は縦横、4:1:1 は横方向に強く色解像度が落ちる。",
  "capture.adc_bits": "ADC の量子化ビット数。少ないほどグラデーションに段 (バンディング) が出る。",
  "capture.codec": "MPEG-2 風の DCT ブロック圧縮を行う。ブロックノイズやモスキートノイズが出る。",
  "capture.bitrate_kbps": "目標ビットレート (29.97 fps 換算)。低いほどブロックノイズが増える。",
  "capture.gop": "I フレームの間隔。長いと I フレームで画質が周期的に切り替わる (ポンピング)。",
  "capture.motion_search": "動き補償の探索範囲。0 で動き補償なし (動きのある部分が崩れやすい)。",
  "capture.rate_control": "レート制御の追従の速さ。大きいほど場面ごとに画質が「呼吸」するように変わる。",
  "capture.frame_drop_rate": "コマ落ち (前フレームの繰り返し) の確率。",

  // 音声
  "audio.enabled": "書き出し時に音声トラックへ劣化を適用する。",
  "audio.mode":
    "リニア: 固定ヘッドのノーマル音声。モノラル・狭帯域でヒスやワウフラッタがある。\n" +
    "Hi-Fi: 回転ヘッドの FM 音声。ステレオ・広帯域だがヘッド切替バズが出る。",
  "audio.low_cut_hz": "リニアトラックの低域カットオフ。大きいほど低音が痩せる。",
  "audio.high_cut_hz": "リニアトラックの高域カットオフ。SP ≒ 10 kHz、3倍モード ≒ 5 kHz。",
  "audio.drive_db": "録音レベル。上げるとテープ飽和で歪む。",
  "audio.hiss_db": "テープヒス (サーッというノイズ) の量。",
  "audio.wow_pct": "ゆっくりした速度変動による音程の揺れ。",
  "audio.flutter_pct": "速い速度変動による音のざらつき・震え。",
  "audio.dropout_rate": "音が一瞬こもって痩せるドロップアウトの頻度 (回/秒)。",
  "audio.hifi_buzz": "Hi-Fi 音声のヘッド切替バズ (ブーンという雑音) の量。トラッキングずれでも増える。",
  "audio.hum_db": "電源ハム (グラウンドループによるブーン音) の量。",
  "audio.hum_hz": "電源周波数。東日本 50 Hz、西日本・北米 60 Hz。",
  "audio.agc": "自動録音レベルの強さ。音量のポンピングや無音部のノイズの持ち上がりが起きる。",
  "audio.sample_rate_hz": "キャプチャ ADC のサンプリング周波数。低いほど高音がこもる。",
  "audio.bits": "キャプチャ ADC の量子化ビット数。少ないほど量子化ノイズが増える。",
  "audio.mono": "音声をモノラルにまとめる。",
  "audio.codec_kbps": "書き出し時の AAC ビットレート。低いほど圧縮ノイズが増える。",
};

export function getPath(obj: any, path: string): any {
  return path.split(".").reduce((o, k) => o?.[k], obj);
}

export function setPath(obj: any, path: string, value: any) {
  const keys = path.split(".");
  const last = keys.pop()!;
  const target = keys.reduce((o, k) => o[k], obj);
  target[last] = value;
}

type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K] };

// 組み込みプリセット (既定値からの差分)
export const presets: { name: string; diff: DeepPartial<Params> }[] = [
  { name: "VHS 標準 (SP)", diff: {} },
  {
    name: "VHS 3倍 (EP)",
    diff: {
      tape: { luma_bw_mhz: 2.2, luma_noise_ire: 4, chroma_bw_khz: 350, chroma_noise_ire: 2.5, chroma_phase_noise_deg: 6, dropout_rate: 0.004 },
      transport: { jitter_samples: 0.8, head_switch_offset: 16 },
      audio: { high_cut_hz: 6000, hiss_db: -46, wow_pct: 0.2, flutter_pct: 0.12 },
    },
  },
  {
    name: "劣化したテープ",
    diff: {
      tape: { luma_bw_mhz: 2.0, luma_noise_ire: 6, chroma_bw_khz: 300, chroma_noise_ire: 4, chroma_phase_noise_deg: 10, saturation_ire: 25, dropout_rate: 0.02, dropout_length: 80, crease_rate: 2, head_clog_rate: 0.5 },
      transport: { jitter_samples: 1.5, wow_samples: 2, head_switch_lines: 8, head_switch_offset: 20, tracking_error: 0.35, vertical_jitter: 0.6 },
      audio: { high_cut_hz: 7000, hiss_db: -44, wow_pct: 0.35, flutter_pct: 0.2, dropout_rate: 0.4, drive_db: 4 },
    },
  },
  {
    name: "Hi-Fi デッキ",
    diff: { audio: { mode: "hifi", hifi_buzz: 0.15 } },
  },
  {
    name: "RF 接続のブラウン管",
    diff: {
      rf: { enabled: true, snow_ire: 4, ghost_gain: 0.15, interference_ire: 1.5, sound_beat: 0.4 },
      audio: { hum_db: -62 },
    },
  },
  {
    name: "DVD レコーダーでダビング",
    diff: {
      capture: { enabled: true, bitrate_kbps: 4500, chroma_subsampling: "420", gop: 15 },
      audio: { codec_kbps: 192 },
    },
  },
  {
    name: "2000年代の取り込み動画 (低ビットレート)",
    diff: {
      transport: { vertical_jitter: 0.3 },
      capture: { enabled: true, levels: "washed_out", bitrate_kbps: 1200, gop: 30, frame_drop_rate: 0.01 },
      audio: { agc: 0.6, hum_db: -58, sample_rate_hz: 32000, codec_kbps: 96 },
    },
  },
  {
    name: "転載を重ねたネット動画",
    diff: {
      capture: { enabled: true, chroma_subsampling: "420", bitrate_kbps: 450, gop: 60, rate_control: 0.8, frame_drop_rate: 0.03 },
      audio: { agc: 0.8, sample_rate_hz: 22000, mono: true, codec_kbps: 32 },
    },
  },
  {
    name: "放送受信 (テープなし)",
    diff: { tape: { enabled: false } },
  },
];

export function applyDiff(base: any, diff: any): any {
  const out = structuredClone(base);
  const rec = (o: any, d: any) => {
    for (const [k, v] of Object.entries(d)) {
      if (v && typeof v === "object" && !Array.isArray(v)) rec(o[k], v);
      else o[k] = v;
    }
  };
  rec(out, diff);
  return out;
}
