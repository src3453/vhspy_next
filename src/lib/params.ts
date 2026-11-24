// vhs-core の Params と対応する型と、GUI 用のパラメータ定義。
// Rust 側 (crates/vhs-core/src/params.rs) を変更したらここも合わせる。

export type Standard = "ntsc" | "pal";
export type TapeFormat = "vhs" | "svhs" | "beta" | "super_beta" | "video8" | "hi8" | "dv" | "digital8" | "custom";
/** DV 方式 (デジタル記録) のフォーマット */
type DigitalFormat = "dv" | "digital8";
export const isDigitalFormat = (f: TapeFormat): f is DigitalFormat => f === "dv" || f === "digital8";

export type Params = {
  /** テレビ方式 */
  standard: Standard;
  /** 入力映像の画質調整 (カメラの有無に関わらずすべての映像に掛かる) */
  input: {
    enabled: boolean;
    blur_px: number;
    sharpness: number;
    sharpness_radius_px: number;
    sharpness_vertical: number;
    sharpness_coring: number;
    sharpness_limit: number;
    brightness: number;
    contrast: number;
    gamma: number;
    saturation: number;
    hue_deg: number;
    color_temp: number;
    tint: number;
  };
  camera: {
    enabled: boolean;
    exposure_ev: number;
    crosstalk: number;
    flare: number;
    registration_px: number;
    shading: number;
    resolution_px: number;
    target_gamma: number;
    dark_current: number;
    photo_lag: number;
    photo_lag_fields: number;
    capacitive_lag: number;
    bias_light: number;
    beam_current: number;
    target_saturation: number;
    act: boolean;
    act_level: number;
    hysteresis: number;
    hysteresis_fields: number;
    burn_rate: number;
    burn_recovery: number;
    blemishes: number;
    amp_drive: number;
    amp_asymmetry: number;
    noise_pct: number;
    mismatch: number;
    aging: number;
    aperture: number;
    knee: number;
    knee_slope: number;
    white_clip: number;
    camera_gamma: number;
  };
  encoder: { enabled: boolean; luma_bw_mhz: number; i_bw_mhz: number; q_bw_mhz: number; uv_bw_mhz: number; chroma_gain: number };
  tape: {
    enabled: boolean;
    format: TapeFormat;
    luma_model: "simple" | "fm";
    fm_sync_mhz: number;
    fm_white_mhz: number;
    fm_channel_mhz: number;
    fm_noise: number;
    color_under_khz: number;
    color_under_level: number;
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
    rf_saturation: number;
    dropout_rate: number;
    dropout_length: number;
    dropout_compensation: boolean;
    crease_rate: number;
    crease_fields: number;
    head_clog_rate: number;
  };
  dv: {
    dct_248: boolean;
    bitrate_scale: number;
    error_rate: number;
    burst_rate: number;
    burst_length: number;
    noise_rate: number;
    noise_length: number;
    noise_density: number;
    ecc: boolean;
    ecc_miss: number;
    head_clog_rate: number;
    head_clog_frames: number;
    concealment: boolean;
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
  tbc: {
    enabled: boolean;
    window_samples: number;
    gate_samples: number;
    reacquire_lines: number;
    error_rate: number;
    residual_samples: number;
    bits: number;
    regenerate_sync: boolean;
  };
  rf: {
    enabled: boolean;
    placement: "before_tape" | "after_tape" | "both";
    snow_ire: number;
    ghost_delay_us: number;
    ghost_gain: number;
    interference_ire: number;
    interference_mhz: number;
    sound_beat: number;
  };
  decoder: {
    yc_separation: "notch" | "comb" | "comb3d";
    motion_threshold_ire: number;
    motion_adaptive: boolean;
    frame_buffer_bits: number;
    chroma_bw_mhz: number;
    acc: boolean;
    color_killer: boolean;
    h_pll: number;
    burst_pll: number;
    pal_delay_line: boolean;
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
  output: { deinterlace: "weave" | "bob"; connection: "composite" | "s_video" };
  osd: {
    enabled: boolean;
    mode: "composite" | "tape";
    markup: string;
    scale: number;
    opacity: number;
    start_datetime: string;
    font: string;
    font_index: number;
    font_dots: number;
  };
  /** ブラウン管 (RetroArch slang シェーダー)。GUI 側の専用パネルで操作する */
  crt: {
    enabled: boolean;
    /** .slangp のパス */
    preset: string;
    /** 録画・書き出しの高さ [px] (幅は 4:3) */
    height: number;
    /** シェーダーパラメータの上書き */
    params: Record<string, number>;
  };
  /** 静止画入力の配置・パン */
  image: {
    fit: "stretch" | "cover" | "contain";
    zoom: number;
    pan_x: number;
    pan_y: number;
    speed_x: number;
    speed_y: number;
    exposure_ev: number;
  };
  seed: number;
};

export type Control =
  | { kind: "range"; path: string; label: string; min: number; max: number; step: number; unit?: string }
  | { kind: "bool"; path: string; label: string }
  | { kind: "select"; path: string; label: string; options: { value: string; label: string }[] }
  | { kind: "text"; path: string; label: string; placeholder?: string }
  | { kind: "markup"; path: string; label: string }
  | { kind: "font"; path: string; label: string };

export type Group = { title: string; note?: string; controls: Control[] };

const r = (path: string, label: string, min: number, max: number, step: number, unit?: string): Control => ({
  kind: "range", path, label, min, max, step, unit,
});
const b = (path: string, label: string): Control => ({ kind: "bool", path, label });

export const groups: Group[] = [
  {
    title: "テレビ方式",
    note: "方式を変えると画像サイズ (NTSC 754×480 / PAL 922×576) とフィールド周波数が変わる。録画中なら録画は止まる。",
    controls: [
      {
        kind: "select", path: "standard", label: "方式",
        options: [
          { value: "ntsc", label: "NTSC (525 / 59.94 Hz)" },
          { value: "pal", label: "PAL-B/G (625 / 50 Hz)" },
        ],
      },
    ],
  },
  {
    title: "画像入力 (静止画)",
    note: "画像ファイルを開いているときだけ効く。速さを 0 以外にすると端で折り返しながらパンし続ける。",
    controls: [
      {
        kind: "select", path: "image.fit", label: "配置",
        options: [
          { value: "stretch", label: "引き伸ばし (縦横比を無視)" },
          { value: "cover", label: "画面を覆う (はみ出しはパンで見る)" },
          { value: "contain", label: "画面に収める (余白は黒)" },
        ],
      },
      r("image.zoom", "拡大", 1, 4, 0.01, "×"),
      r("image.pan_x", "横位置", -1, 1, 0.01),
      r("image.pan_y", "縦位置", -1, 1, 0.01),
      r("image.speed_x", "横パン速度", -1, 1, 0.005, "画面/s"),
      r("image.speed_y", "縦パン速度", -1, 1, 0.005, "画面/s"),
      r("image.exposure_ev", "HDR 露出", -8, 8, 0.1, "EV"),
    ],
  },
  {
    title: "入力の画質調整",
    note: "入力映像すべてに最初に掛かる (撮像管カメラ・テープより前)。昔のビデオカメラの甘いレンズ、強い輪郭強調のハロー、色の癖などを作る。",
    controls: [
      b("input.enabled", "画質調整を有効にする"),
      r("input.blur_px", "ぼけ (レンズ)", 0, 4, 0.05, "px"),
      r("input.sharpness", "シャープネス", -1, 4, 0.01),
      r("input.sharpness_radius_px", "輪郭強調の太さ", 0.3, 4, 0.05, "px"),
      r("input.sharpness_vertical", "強調の方向 (横のみ ↔ 縦横)", 0, 1, 0.01),
      r("input.sharpness_coring", "コアリング", 0, 0.1, 0.001),
      r("input.sharpness_limit", "強調の上限", 0, 0.5, 0.005),
      r("input.brightness", "明るさ", -0.5, 0.5, 0.005),
      r("input.contrast", "コントラスト", 0, 2, 0.01),
      r("input.gamma", "ガンマ", 0.3, 3, 0.01),
      r("input.saturation", "彩度", 0, 2, 0.01),
      r("input.hue_deg", "色相", -180, 180, 1, "°"),
      r("input.color_temp", "色温度 (青 ↔ 赤)", -1, 1, 0.01),
      r("input.tint", "色かぶり (緑 ↔ マゼンタ)", -1, 1, 0.01),
    ],
  },
  {
    title: "撮像管カメラ: 光学系",
    note: "入力映像を被写体とみなし、RGB 3 管式の撮像管カメラで撮影した映像にする (パイプラインの先頭)。",
    controls: [
      b("camera.enabled", "撮像管カメラを通す"),
      r("camera.exposure_ev", "露出 (アイリス)", -4, 4, 0.1, "EV"),
      r("camera.crosstalk", "プリズムの混色", 0, 0.3, 0.005),
      r("camera.flare", "フレア", 0, 0.2, 0.005),
      r("camera.registration_px", "レジストレーションずれ", 0, 6, 0.05, "px"),
      r("camera.shading", "シェーディング", 0, 0.5, 0.01),
      r("camera.resolution_px", "ぼけ (ビームスポット)", 0, 4, 0.05, "px"),
    ],
  },
  {
    title: "撮像管カメラ: ターゲット・ビーム",
    note: "ビジコン: γ≒0.65・残像大・焼き付きやすい。プランビコン: γ≒0.95・残像小。サチコン: γ≒1.0。",
    controls: [
      r("camera.target_gamma", "ターゲット γ", 0.4, 1.2, 0.01),
      r("camera.dark_current", "暗電流", 0, 0.1, 0.001),
      r("camera.capacitive_lag", "容量性残像", 0, 2, 0.01),
      r("camera.photo_lag", "光導電性残像", 0, 0.9, 0.01),
      r("camera.photo_lag_fields", "残像の時定数", 0.5, 60, 0.5, "field"),
      r("camera.bias_light", "バイアス光", 0, 0.2, 0.002),
      r("camera.beam_current", "ビーム電流", 0.5, 6, 0.05, "×白"),
      r("camera.target_saturation", "ターゲット飽和", 1, 40, 0.5, "×白"),
      b("camera.act", "ACT (アンチコメットテール)"),
      r("camera.act_level", "ACT レベル", 0.5, 5, 0.05, "×白"),
      r("camera.hysteresis", "ヒステリシス幅", 0, 0.2, 0.002),
      r("camera.hysteresis_fields", "ヒステリシス緩和", 1, 300, 1, "field"),
      r("camera.burn_rate", "焼き付き速度", 0, 1, 0.005, "/s"),
      r("camera.burn_recovery", "焼き付き回復", 0, 0.5, 0.005, "/s"),
      r("camera.blemishes", "しみ・きず", 0, 200, 1),
    ],
  },
  {
    title: "撮像管カメラ: 増幅・プロセス",
    controls: [
      r("camera.amp_drive", "プリアンプ非線形", 0, 3, 0.01),
      r("camera.amp_asymmetry", "正負の非対称", -1, 1, 0.01),
      r("camera.noise_pct", "プリアンプ雑音", 0, 10, 0.05, "%"),
      r("camera.mismatch", "RGB 管のばらつき", 0, 1, 0.01),
      r("camera.aging", "撮像管の劣化", 0, 1, 0.01),
      r("camera.aperture", "輪郭補償", 0, 2, 0.01),
      r("camera.knee", "ニー開始", 0.5, 1.2, 0.01),
      r("camera.knee_slope", "ニー傾き", 0, 1, 0.01),
      r("camera.white_clip", "ホワイトクリップ", 0.8, 1.5, 0.01),
      r("camera.camera_gamma", "ガンマ補正", 0.3, 1, 0.01),
    ],
  },
  {
    title: "OSD (画面表示)",
    note:
      "タグ: <color=色> <bg=色> <edge=色> <shadow=色> <size=1〜8> <blink> は </名前> で閉じる。" +
      "<pos=X,Y> (NTSC 754×480 / PAL 922×576 座標) と <align=left|center|right> は単独。" +
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
      { kind: "font", path: "osd.font", label: "フォント" },
      r("osd.font_index", "TTC 内の番号", 0, 7, 1),
      r("osd.font_dots", "ドット数 (0=なめらか)", 0, 32, 1, "dot"),
      r("osd.scale", "文字サイズ", 1, 8, 1),
      r("osd.opacity", "不透明度", 0, 1, 0.01),
      { kind: "text", path: "osd.start_datetime", label: "開始日時", placeholder: "1995-08-12 14:30:00" },
    ],
  },
  {
    title: "テープ: フォーマット",
    note:
      "フォーマットを選ぶと搬送波周波数・低域変換周波数・帯域などをまとめて設定し、FM モデルに切り替える " +
      "(テレビ方式を変えたときも、その方式での値に合わせ直す)。",
    controls: [
      b("tape.enabled", "テープを通す"),
      {
        kind: "select", path: "tape.format", label: "フォーマット",
        options: [
          { value: "vhs", label: "VHS" },
          { value: "svhs", label: "S-VHS" },
          { value: "beta", label: "ベータ (β-II)" },
          { value: "super_beta", label: "SuperBeta" },
          { value: "video8", label: "Video8" },
          { value: "hi8", label: "Hi8" },
          { value: "dv", label: "DV (MiniDV)" },
          { value: "digital8", label: "Digital8" },
          { value: "custom", label: "カスタム" },
        ],
      },
      {
        kind: "select", path: "tape.luma_model", label: "輝度の FM モデル",
        options: [
          { value: "fm", label: "FM を計算 (変調・伝送路・復調)" },
          { value: "simple", label: "簡易 (加算ノイズ・確率的ベアディング)" },
        ],
      },
      r("tape.fm_sync_mhz", "シンクチップ周波数", 2, 8, 0.05, "MHz"),
      r("tape.fm_white_mhz", "ホワイトピーク周波数", 2.5, 10, 0.05, "MHz"),
      r("tape.fm_channel_mhz", "再生伝送路の帯域", 2.5, 12, 0.1, "MHz"),
      r("tape.fm_noise", "RF ノイズ", 0, 0.5, 0.005),
      r("tape.color_under_khz", "低域変換周波数", 400, 1200, 1, "kHz"),
      r("tape.color_under_level", "低域変換の記録レベル", 0, 0.6, 0.01),
      {
        kind: "select", path: "output.connection", label: "受像機への接続",
        options: [
          { value: "composite", label: "コンポジット (映像端子)" },
          { value: "s_video", label: "S 映像 (Y/C 分離)" },
        ],
      },
    ],
  },
  {
    title: "DV (MiniDV / Digital8)",
    note:
      "フォーマットが DV / Digital8 のとき、アナログのテープ処理の代わりに使う。25 Mbps・4:1:1 (PAL は 4:2:0) の " +
      "DCT 圧縮で、5 マクロブロックごとに符号量が一定なので、細かい絵柄の周りだけブロックノイズが出る。" +
      "「コンポジット信号を通す」をオフにすると FireWire 取り込み (デジタルのまま)、オンならデッキのアナログ出力。",
    controls: [
      r("dv.bitrate_scale", "符号量 (1 = 25 Mbps)", 0.1, 2, 0.01, "×"),
      b("dv.dct_248", "動きのあるブロックを 2-4-8 DCT"),
      r("dv.noise_rate", "バーストノイズ (データ化け)", 0, 60, 0.1, "回/s"),
      r("dv.noise_length", "バーストノイズの長さ", 1, 400, 1, "MB"),
      r("dv.noise_density", "区間内のバイト誤り率", 0, 0.5, 0.001),
      b("dv.ecc", "誤り訂正 (ECC)"),
      r("dv.ecc_miss", "誤りの見逃し", 0, 1, 0.01),
      b("dv.concealment", "エラー修整 (前のフレームで埋める)"),
      r("dv.error_rate", "ブロックの読み取りエラー", 0, 0.05, 0.0005),
      r("dv.burst_rate", "ドロップアウト (読めない区間)", 0, 30, 0.1, "回/s"),
      r("dv.burst_length", "ドロップアウトの長さ", 1, 27, 1, "seg"),
      r("dv.head_clog_rate", "ヘッド目詰まり", 0, 30, 0.5, "回/分"),
      r("dv.head_clog_frames", "目詰まりの長さ", 1, 120, 1, "frame"),
    ],
  },
  {
    title: "テープ: 輝度 (FM 記録)",
    controls: [
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
    ],
  },
  {
    title: "テープ: 磁性体",
    note: "記録するすべての信号 (FM 輝度の RF・低域変換クロマ・リニア音声) に同じ伝達特性が掛かる。",
    controls: [
      {
        kind: "select", path: "tape.magnetic_model", label: "磁性体モデル",
        options: [{ value: "tanh", label: "tanh 飽和" }, { value: "linear", label: "線形" }],
      },
      r("tape.saturation_ire", "色の飽和レベル", 5, 150, 1, "IRE"),
      r("tape.rf_saturation", "RF の飽和レベル", 0.2, 5, 0.05),
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
    title: "TBC (タイムベースコレクタ)",
    note: "再生信号をラインメモリに取り込み、同期を検出して時間軸の揺れを取り除く。同期を見失う・誤検出するとラインが横に飛ぶ (TBC エラー)。",
    controls: [
      b("tbc.enabled", "TBC を通す"),
      r("tbc.window_samples", "補正範囲", 0, 80, 0.5, "smp"),
      r("tbc.gate_samples", "同期ゲート幅", 0.5, 40, 0.5, "smp"),
      r("tbc.reacquire_lines", "再捕捉までのライン数", 0, 30, 1, "line"),
      r("tbc.error_rate", "同期の誤検出", 0, 0.05, 0.0005),
      r("tbc.residual_samples", "残留ジッタ", 0, 1, 0.01, "smp"),
      r("tbc.bits", "A/D ビット数 (0 = なし)", 0, 12, 1, "bit"),
      b("tbc.regenerate_sync", "同期を作り直す"),
    ],
  },
  {
    title: "RF 接続 (アンテナ端子)",
    note: "チューナ経由で受けたときの劣化。放送受信 (録画前) / デッキ→テレビ (再生後) / 両方を選べる。",
    controls: [
      b("rf.enabled", "RF 経由で受像"),
      {
        kind: "select", path: "rf.placement", label: "経路",
        options: [
          { value: "after_tape", label: "デッキ → テレビ (テープの後)" },
          { value: "before_tape", label: "放送受信 → 録画 (テープの前)" },
          { value: "both", label: "両方 (放送を録画して RF で再生)" },
        ],
      },
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
    note: "コンポジット信号を通さないと、カメラ・OSD・DV・キャプチャだけを通す (アナログのテープ・RF・TBC・受像機は飛ばす)。",
    controls: [
      b("encoder.enabled", "コンポジット信号 (NTSC / PAL) を通す"),
      r("encoder.luma_bw_mhz", "輝度帯域", 1, 6, 0.05, "MHz"),
      r("encoder.i_bw_mhz", "I 帯域", 0.2, 3, 0.05, "MHz"),
      r("encoder.q_bw_mhz", "Q 帯域", 0.1, 3, 0.05, "MHz"),
      r("encoder.uv_bw_mhz", "U/V 帯域 (PAL)", 0.2, 3, 0.05, "MHz"),
      r("encoder.chroma_gain", "色ゲイン", 0, 3, 0.01),
    ],
  },
  {
    title: "受像機 (デコーダ)",
    controls: [
      {
        kind: "select", path: "decoder.yc_separation", label: "Y/C 分離",
        options: [
          { value: "notch", label: "ノッチ (ドットクロール)" },
          { value: "comb", label: "くし形 (NTSC 1H / PAL 2H)" },
          { value: "comb3d", label: "3 次元 (画面バッファ)" },
        ],
      },
      b("decoder.motion_adaptive", "3D: 動き適応"),
      r("decoder.motion_threshold_ire", "3D: 動き検出しきい値", 0.5, 20, 0.5, "IRE"),
      r("decoder.frame_buffer_bits", "3D: バッファのビット数 (0 = なし)", 0, 12, 1, "bit"),
      r("decoder.chroma_bw_mhz", "色差帯域", 0.2, 3, 0.05, "MHz"),
      b("decoder.acc", "ACC"),
      b("decoder.color_killer", "カラーキラー"),
      r("decoder.h_pll", "水平 AFC 追従", 0, 1, 0.01),
      r("decoder.burst_pll", "色副搬送波 PLL 追従", 0.02, 1, 0.01),
      b("decoder.pal_delay_line", "PAL 遅延線"),
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
  // 画像入力
  "image.fit": "画像を画面 (4:3) に合わせる方法。「覆う」や拡大で画面からはみ出した部分を、位置・パンで見る。",
  "image.zoom": "配置した大きさからの拡大率。",
  "image.pan_x": "表示位置。-1 で左端、1 で右端が見える (画像が画面より小さいときは画像が左右に寄る)。",
  "image.pan_y": "表示位置。-1 で上端、1 で下端が見える。",
  "image.speed_x": "横方向にカメラを振る速さ (1 秒に動く画面幅)。端で折り返す。動きでくし形 Y/C 分離・残像・圧縮の劣化が見える。",
  "image.speed_y": "縦方向にカメラを振る速さ (1 秒に動く画面高さ)。端で折り返す。",
  "image.exposure_ev": "OpenEXR などの HDR 画像 (シーンリニア) に掛ける露出。8 bit の画像には効かない。白を超える光は撮像管カメラでコメットテールなどになる。",
  // 入力の画質調整
  "input.enabled": "入力映像の画質調整を有効にする。撮像管カメラの有無に関わらず、パイプラインの先頭ですべての映像に掛かる。",
  "input.blur_px": "レンズや光学ローパスフィルタによるぼけ (ガウスの σ)。家庭用ビデオカメラの甘い解像感。",
  "input.sharpness": "輝度の輪郭強調 (アンシャープマスク)。上げると輪郭の両側に白・黒の縁 (ハロー) が出る。負の値で細部を削る。",
  "input.sharpness_radius_px": "輪郭強調で見る範囲。大きいほどハローが太く、昔のカメラらしいにじんだ縁になる。",
  "input.sharpness_vertical": "輪郭強調の異方性。0 で横方向だけ (縦の線の縁にだけハローが出る。1H 遅延線の垂直補正がない家庭用機)、1 で縦横同じ。間は縦方向の強調が線形に強まる。",
  "input.sharpness_coring": "これより小さい細部は強調しない。雑音が強調でざらつくのを抑える。",
  "input.sharpness_limit": "輪郭強調で足す明るさの上限。0 で無制限。",
  "input.brightness": "明るさ (全体に足す量)。",
  "input.contrast": "中間の明るさを中心にした階調の傾き。",
  "input.gamma": "中間調の明るさ。1 より大きいと明るく、小さいと暗く沈む。",
  "input.saturation": "色の濃さ。",
  "input.hue_deg": "色相の回転。",
  "input.color_temp": "ホワイトバランスのずれ。負で青っぽく (蛍光灯・日陰)、正で赤っぽく (白熱灯) なる。明るさは保つ。",
  "input.tint": "色かぶり。負で緑、正でマゼンタに寄る。",
  // 撮像管カメラ
  "camera.enabled": "入力映像を撮像管カメラで撮影した映像にする。テープ・OSD の焼き込みより前に入る。",
  "camera.exposure_ev": "アイリスによる露出。上げると白を超えるハイライトが増え、コメットテールや焼き付きが起きやすい。",
  "camera.crosstalk": "色分解プリズムの分光特性の裾による隣の色の漏れ込み。色が浅くなる。",
  "camera.flare": "レンズ・プリズム内の散乱光が画面全体にかぶる量。暗部が浮き、赤管は特に多く暗部の色が赤に転ぶ。",
  "camera.registration_px": "RGB 3 管の像の重ね合わせのずれ。G 管を基準に R/B がずれて輪郭に色がにじむ。画面端ほど大きくなる。",
  "camera.shading": "画面周辺の感度低下と、管ごとに異なる傾き (色むら)。",
  "camera.resolution_px": "レンズ・ターゲット・電子ビームのスポット径によるぼけ。",
  "camera.target_gamma": "光導電ターゲットの光電変換特性。ビジコン ≒ 0.65、プランビコン ≒ 0.95、サチコン ≒ 1.0。ガンマ補正と組み合わせて全体の階調が決まる。",
  "camera.dark_current": "光がなくても流れる電流。オートブラックで平均は差し引かれるが、むらや白きずが残る。",
  "camera.capacitive_lag": "ビームが 1 回で電荷を読み切れないことによる残像。暗い部分ほど尾を引く。",
  "camera.photo_lag": "光導電体のトラップによる遅い残像の割合。動く被写体が尾を引き、消えるまで時間がかかる。",
  "camera.photo_lag_fields": "光導電性残像の減衰の時定数 (立ち上がりはこの半分)。60 で約 1 秒。",
  "camera.bias_light": "ターゲットに一様な光を当てて暗部の残像を減らす (バイアス光)。黒レベルは差し引かれる。",
  "camera.beam_current": "1 回の走査で読み出せる電荷の上限 (白レベル比)。これを超える明るい部分は読み残され、動くと彗星のような尾 (コメットテール) を引く。",
  "camera.target_saturation": "ターゲットに溜められる電荷の上限。大きいほど強いハイライトの尾が長くなる。",
  "camera.act": "ACT 管: 帰線期間に過大な電荷を放電してコメットテールを短くする。",
  "camera.act_level": "ACT が放電を始める電荷のレベル。",
  "camera.hysteresis": "ターゲットのヒステリシス。明→暗と暗→明で残る値が異なり、動いた後に薄い像が残る。",
  "camera.hysteresis_fields": "ヒステリシスの残りが解ける時定数。",
  "camera.burn_rate": "明るい部分の感度が落ちて像が焼き付く速さ。カメラを動かすと前の像が暗く残る。",
  "camera.burn_recovery": "焼き付きが回復する速さ。",
  "camera.blemishes": "ターゲットのしみ (黒い点) と白きず。管ごとに別の場所に出るので色が付く。",
  "camera.amp_drive": "初段の真空管プリアンプの非線形性の強さ。0 で線形。",
  "camera.amp_asymmetry": "正側と負側の飽和の非対称性。正で明部、負で暗部がより圧縮される (偶数次ひずみ)。",
  "camera.noise_pct": "プリアンプの雑音。入力容量のため高域ほど大きい (三角雑音)。ガンマ補正で暗部ほど目立つ。",
  "camera.mismatch": "RGB 3 本の撮像管の特性のばらつき。γ・感度・残像・暗電流・ビーム電流などが管ごとにずれ、残像や暗部に色が付く。",
  "camera.aging": "撮像管の劣化をまとめて進める。感度とビーム電流の低下、残像・焼き付き・暗電流・しみ・雑音・ばらつきの増加。",
  "camera.aperture": "輪郭補償 (アパーチャ補正)。撮像管の解像度不足を電気的に補う。上げると輪郭に縁取りが出る。",
  "camera.knee": "ハイライト圧縮 (ニー) を始めるレベル。",
  "camera.knee_slope": "ニーより上の傾き。小さいほどハイライトが強く圧縮される。",
  "camera.white_clip": "ホワイトクリップのレベル。",
  "camera.camera_gamma": "カメラのガンマ補正。ターゲット γ × ガンマ補正 ≒ 0.45 で元の階調になる。",

  // OSD
  "osd.enabled": "デッキやカメラの画面表示 (PLAY、カウンタ、日付など) を重ねる。",
  "osd.mode":
    "デッキ出力に重畳: 再生機が出力に文字を重ねる (くっきり、テープ劣化を受けない)。\n" +
    "テープに焼き込み: 録画時に記録された文字 (カメラの日付など)。テープの劣化を一緒に受ける。",
  "osd.markup": "表示内容のマークアップ。書式はグループ上部の説明を参照。",
  "osd.scale": "既定の文字サイズ。1 = 1 ドットが 2×2 画素。<size=> タグで部分的に変更できる。",
  "osd.opacity": "OSD 全体の不透明度。0 で透明、1 で不透明。",
  "osd.font":
    "表示に使うフォントファイル (.ttf / .otf / .ttc)。空なら内蔵の 5×7 ドットフォント。\n" +
    "例: C:\\Windows\\Fonts\\msgothic.ttc (日本語も表示できる)。フォントにない文字は内蔵フォントで代用する。",
  "osd.font_index": ".ttc (フォントコレクション) の中のどのフォントを使うか。msgothic.ttc なら 0 = MS ゴシック、2 = MS P ゴシック。",
  "osd.font_dots":
    "外部フォントを 1 文字の高さあたり何ドットで描くか。16 で漢字 ROM 風のドット文字になる。0 はなめらかに描く。内蔵フォントには効かない。",
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
  "tape.magnetic_model":
    "磁性体の伝達特性。記録するすべての信号に掛かる。tanh は記録レベルが大きいと磁化が飽和する: " +
    "強い色が頭打ちになり、FM の搬送波と色の相互変調で鮮やかな色の部分に輝度のモアレが出て、" +
    "リニア音声は大きい音が潰れる (テープコンプレッション)。線形は飽和なし。",
  "tape.saturation_ire": "低域変換クロマの磁気飽和が始まる色の振幅。小さいほど鮮やかな色が早く潰れる (tanh のみ)。",
  "tape.rf_saturation":
    "FM 輝度の RF (搬送波の振幅 = 1) の飽和レベル。小さいほど搬送波と色の相互変調が強くなり、" +
    "鮮やかな色の部分に細かいモアレが出る。搬送波も縮むのでノイズがやや増える (tanh のみ)。",

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

  // TBC
  "tbc.enabled": "再生機のタイムベースコレクタ。ジッタ・ワウフラッタ・スキュー・ヘッド切替の曲がりを取り除く。",
  "tbc.window_samples": "補正できる時間軸のずれ (ラインメモリの余裕)。超えた分はそのまま残る。",
  "tbc.gate_samples":
    "前のラインの同期位置からこの範囲だけで次の同期を探す。狭いと急なずれ (ヘッド切替・ノイズバー) で同期を見失い、" +
    "前のラインの補正量のままになってラインがずれる。",
  "tbc.reacquire_lines": "同期を見失ってから探す範囲を広げて捕まえ直すまでのライン数。大きいほどずれたラインが長く続く。",
  "tbc.error_rate": "ノイズで同期を誤検出する確率 (1 ラインあたり)。誤った位置が基準になるので、数ライン続けて横に飛ぶ。",
  "tbc.residual_samples": "補正後に残る書き込みクロックの揺れ (rms)。",
  "tbc.bits": "ラインメモリの A/D 変換のビット数。小さいほど階調が粗くなる。0 で量子化しない。",
  "tbc.regenerate_sync":
    "同期・ブランキングを作り直して出力する。TBC が誤ったずれは受像機の AFC でももう直らない。" +
    "オフだと元の同期をそのまま通す (受像機の AFC が TBC の誤りを一部打ち消す)。",

  // RF
  "rf.enabled": "デッキの RF 出力 (アンテナ端子) をテレビのチューナで受けた状態にする。",
  "rf.placement":
    "デッキ → テレビ: 再生したデッキの RF 出力をテレビで受ける (テープの劣化の後に乗る)。\n" +
    "放送受信 → 録画: 電波で受けた放送を録画した状態 (スノーやゴーストもテープに記録され、テープの劣化を一緒に受ける)。\n" +
    "両方: 放送を録画したテープを RF 接続で見る。2 回の雑音は独立。",
  "rf.snow_ire": "受信ノイズ (スノー) の量。電波が弱いときの砂嵐。",
  "rf.ghost_gain": "反射波によるゴースト (二重像) の強さ。負にすると反転ゴースト。",
  "rf.ghost_delay_us": "ゴーストの遅れ。大きいほど二重像が右に離れる。",
  "rf.interference_ire": "ビート妨害 (斜めの縞模様) の強さ。",
  "rf.interference_mhz": "ビート妨害の周波数。縞の細かさ・角度が変わる。",
  "rf.sound_beat": "音声搬送波が映像に漏れる量。920 kHz ビートや音声に連動する縞が出る。",

  // エンコーダ
  standard:
    "テレビ方式。NTSC は位相の誤差がそのまま色相のずれになる。PAL は V 成分をラインごとに反転して送るので、" +
    "受像機の遅延線で上下のラインを平均すると位相の誤差が打ち消され、色相ではなく彩度の低下になる。",
  "encoder.luma_bw_mhz": "送出側の輝度帯域。NTSC 放送は 4.2 MHz、PAL-B/G は 5 MHz。",
  "encoder.uv_bw_mhz": "PAL の U/V 信号の帯域。規格は 1.3 MHz。NTSC では使わない。",
  "encoder.i_bw_mhz": "I 信号 (橙〜青緑方向の色差) の帯域。NTSC 規格は 1.3 MHz。",
  "encoder.q_bw_mhz": "Q 信号 (紫〜黄緑方向の色差) の帯域。NTSC 規格は 0.6 MHz。",
  "encoder.chroma_gain": "送出時の色信号の振幅倍率。大きいとテープでの色飽和が起きやすい。",

  // デコーダ
  "decoder.yc_separation":
    "輝度と色の分離方式。ノッチは安価な方式でドットクロールや虹色のクロスカラーが出る。" +
    "くし形はそれらを抑えるが、縦方向の色の境目がにじむ (PAL は 2 ライン離れたラインと比べるので、にじみが大きい)。" +
    "3 次元は画面バッファ (フレームメモリ) に持った前のフレーム (NTSC 1 フレーム / PAL 2 フレーム前) と比べるので、" +
    "静止部分ではクロスカラーもにじみもほぼ出ない。テープ再生のように時間軸や色の位相がフレームごとに揺れると効きが落ちる。",
  "decoder.motion_adaptive":
    "3 次元 Y/C 分離の動き適応。動きを検出した部分だけラインくし形に切り替える。" +
    "切ると常にフレームくし形になり、動く部分に前のフレームの絵が色の縞・二重像として残る。",
  "decoder.motion_threshold_ire":
    "3 次元 Y/C 分離で動きとみなすフレーム間差。小さいと雑音でもラインくし形に逃げ、大きいと動く部分にクロスカラーの残像が出る。",
  "decoder.frame_buffer_bits":
    "3 次元 Y/C 分離の画面バッファのビット数 (同期先端〜ピークを量子化)。少ないと静止部分にも量子化雑音や色の等高線が乗る。0 で量子化しない。",
  "decoder.chroma_bw_mhz": "受像機で復調した後の色差の帯域。小さいほど色がにじむ。",
  "decoder.burst_pll":
    "色副搬送波の PLL がバーストに追従する速さ (ライン単位)。1 = ラインごとに即時同期。" +
    "小さくするとテープのライン単位の位相ゆらぎを取り切れず、NTSC では色相の横縞、PAL では彩度のむらになる。",
  "decoder.pal_delay_line":
    "PAL の 1H 遅延線 (PAL-D)。切るとシンプル PAL になり、位相の誤差がライン交互の色相ずれ (ハノーバーバー) として見える。",
  "decoder.acc": "カラーバーストの振幅で色の濃さを自動調整する (ACC)。",

  // テープのフォーマット
  "tape.format":
    "テープのフォーマット。VHS / ベータ / Video8 は同じ方式 (FM 輝度 + 低域変換クロマ) で、搬送波周波数と帯域が違う。" +
    "S-VHS / Hi8 は搬送波を上げて輝度帯域を広げた高画質版で、S 映像でつなぐのが普通。" +
    "DV (MiniDV) / Digital8 はデジタル記録で、アナログのテープ処理の代わりに「DV」の設定で処理する。",
  "tape.luma_model":
    "FM を計算: 輝度で搬送波を周波数変調し、ヘッド・テープの伝送路とノイズを通して、ゼロ交差の間隔から復調する。" +
    "ベアディング・砂嵐・RF レベルの低下が自然に出る (重い)。簡易: 帯域制限と加算ノイズで近似する (軽い)。",
  "tape.fm_sync_mhz": "シンクチップ (同期の先端) を記録する搬送波周波数。VHS 3.4 MHz、Hi8 5.7 MHz など。",
  "tape.fm_white_mhz": "ホワイトピークを記録する搬送波周波数。シンクチップとの差が周波数偏移。",
  "tape.fm_channel_mhz":
    "再生伝送路 (ヘッド・テープ・再生アンプ) の帯域。狭いと白のエッジで搬送波が弱まり、" +
    "ゼロ交差を見落として白の右に黒い筋 (ベアディング) が出る。ヘッドの摩耗の目安。",
  "tape.fm_noise": "再生 RF のノイズ (搬送波に対する比)。大きいと白黒の粒状ノイズが増え、トラッキングずれで一気に悪化する。",
  "tape.color_under_khz": "低域変換クロマの周波数。VHS 629 kHz、ベータ 688 kHz、8mm 743 kHz (NTSC)。",
  "tape.color_under_level": "低域変換クロマの記録レベル (FM 搬送波に対する比)。大きいと鮮やかな色の部分の輝度にモアレが乗る。",
  "output.connection":
    "再生機と受像機の接続。S 映像は Y と C を分けたまま送るので、受像機の Y/C 分離によるドットクロール・クロスカラーが出ない。" +
    "テープを通さないとき・テープの後に RF でつなぐときはコンポジットになる。",
  "decoder.color_killer": "カラーバーストが弱いとき色を消して白黒表示にする。",
  "decoder.h_pll": "水平同期 (AFC) の追従の速さ。0 = 追従しない、1 = 即時追従。小さいと時間軸の揺れがそのまま見える。",
  "decoder.contrast": "受像機のコントラスト (映像の振幅倍率)。",
  "decoder.brightness": "受像機の明るさ (黒レベルのオフセット)。",
  "decoder.saturation": "受像機の色の濃さ。",
  "decoder.hue_deg": "受像機の色相 (ティント)。NTSC 特有の色ずれを再現・補正できる。",
  "output.deinterlace": "Weave: 2 フィールドを交互に織り込む (動きに櫛状のずれ)。Bob: 各フィールドのラインを 2 倍に伸ばす。",

  // コンポジットの有無
  "encoder.enabled":
    "オフにすると NTSC / PAL の信号を作らず、映像をデジタルのまま流す。撮像管カメラや OSD、DV、キャプチャだけを試すとき用。" +
    "DV と組み合わせると FireWire (i.LINK) 取り込みと同じ経路になる。",

  // DV
  "dv.bitrate_scale":
    "1 セグメント (5 マクロブロック) に割り当てる符号量の倍率。1 が規格どおり (25 Mbps)。" +
    "下げると、細かい絵柄やノイズの多い所のブロックノイズ・モスキートノイズが目立つ。",
  "dv.dct_248":
    "上下のライン (別フィールド) の食い違いが大きいブロックを、2 フィールドの和と差に分けて変換する。" +
    "切ると動きのある所の横縞 (コーミング) を 8×8 のまま符号化して粗くなる。",
  "dv.noise_rate":
    "テープのデータが化けるバーストノイズの頻度。読み取り順に連続したマクロブロックにバイト誤りが乗る。" +
    "誤り訂正で直せない分は修整されるか、見逃されて化けたまま (縞・格子・色のずれたブロック) 復号される。",
  "dv.noise_length": "バーストノイズ 1 回の平均の長さ (マクロブロック数)。長いとトラックをまたいで画面の離れた場所にも散る。",
  "dv.noise_density":
    "バーストノイズの区間で 1 バイトが化ける確率。マクロブロック (77 バイト) あたり 4 バイトまでは誤り訂正で直る。",
  "dv.ecc": "誤り訂正 (内符号のリード・ソロモン)。オフにすると誤りはすべて化けたまま復号される (壊れたファイルの再生に近い)。",
  "dv.ecc_miss": "訂正しきれない誤りを検出し損ねる確率。検出できた誤りは前のフレームで埋められる。",
  "dv.concealment":
    "読めないブロックを前のフレームの同じ位置で埋める。動く映像ではモザイク状の残像になる。" +
    "切ると壊れたブロックがでたらめな色のまま出る。",
  "dv.error_rate": "マクロブロックごとの読み取りエラー (検出されて修整される) の確率 [1/フレーム]。",
  "dv.burst_rate": "テープの傷などでセグメント単位に読めない区間 (ドロップアウト) の頻度。",
  "dv.head_clog_rate":
    "ヘッドの目詰まり。片方のヘッドなら 1 本おきのトラック (画面に散らばったブロック) が、両方なら画面全体が前のフレームで止まる。",

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

// ---- 今の設定で効かないコントロールの判定 ----
// パイプライン (crates/vhs-core/src/pipeline.rs ほか) の分岐に合わせる。向こうを変えたらここも直す。

/** コンポジット信号 (エンコーダ → 受像機) を通すか */
const composite = (p: Params) => p.encoder.enabled;
/** アナログのテープ処理 (FM 輝度・低域変換クロマ・走行系・TBC) を通すか */
const analogTape = (p: Params) => composite(p) && p.tape.enabled && !isDigitalFormat(p.tape.format);
/** DV の記録・再生を通すか (コンポジットを通さなくても FireWire 取り込みとして効く) */
const dvTape = (p: Params) => p.tape.enabled && isDigitalFormat(p.tape.format);
const fmModel = (p: Params) => p.tape.luma_model === "fm";
const tanh = (p: Params) => p.tape.magnetic_model === "tanh";
/** S 映像が実際に使われるか (テープの再生出力を受像機へ直接つなぐときだけ) */
const sVideo = (p: Params) =>
  composite(p) &&
  p.output.connection === "s_video" &&
  p.tape.enabled &&
  !(p.rf.enabled && (p.rf.placement === "after_tape" || p.rf.placement === "both"));
const ycSeparated = (p: Params) => composite(p) && !sVideo(p);
const comb3d = (p: Params) => ycSeparated(p) && p.decoder.yc_separation === "comb3d";

/** path → 効く条件。ここにない path は常に有効。
 *  `"camera."` のように末尾が "." のキーはそのグループ全体 (個別のキーがあればそちらも満たす必要がある) */
const activeWhen: Record<string, (p: Params) => boolean> = {
  // 入力の画質調整
  "input.": (p) => p.input.enabled,
  "input.sharpness_radius_px": (p) => p.input.sharpness !== 0,
  "input.sharpness_vertical": (p) => p.input.sharpness !== 0,
  "input.sharpness_coring": (p) => p.input.sharpness !== 0,
  "input.sharpness_limit": (p) => p.input.sharpness !== 0,

  // 撮像管カメラ
  "camera.": (p) => p.camera.enabled,
  "camera.act_level": (p) => p.camera.act,

  // OSD
  "osd.": (p) => p.osd.enabled,
  "osd.font_index": (p) => /\.(ttc|otc)$/i.test(p.osd.font.trim()),
  "osd.font_dots": (p) => p.osd.font.trim() !== "",

  // テープ (アナログ)
  "tape.": analogTape,
  "tape.format": (p) => p.tape.enabled,
  // 簡易モデルでも磁性体が tanh なら、搬送波と色の相互変調の計算に搬送波・低域変換の値を使う
  "tape.fm_sync_mhz": (p) => fmModel(p) || tanh(p),
  "tape.fm_white_mhz": (p) => fmModel(p) || tanh(p),
  "tape.color_under_khz": (p) => fmModel(p) || tanh(p),
  "tape.color_under_level": (p) => fmModel(p) || tanh(p),
  "tape.fm_channel_mhz": fmModel,
  "tape.fm_noise": fmModel,
  // FM を計算するモデルではノイズ・ベアディングは伝送路から自然に出る
  "tape.luma_noise_ire": (p) => !fmModel(p),
  "tape.bearding_threshold_ire": (p) => !fmModel(p),
  "tape.bearding_strength": (p) => !fmModel(p),
  "tape.saturation_ire": tanh,
  "tape.rf_saturation": tanh,
  "output.connection": (p) => composite(p) && p.tape.enabled && !(p.rf.enabled && p.rf.placement !== "before_tape"),

  // DV
  "dv.": dvTape,
  "dv.ecc_miss": (p) => p.dv.ecc,

  // 走行系・TBC (アナログのテープの再生時だけ)
  "transport.": analogTape,
  // トラッキングずれは Hi-Fi 音声のバズにも効く
  "transport.tracking_error": (p) => analogTape(p) || (p.audio.enabled && p.audio.mode === "hifi"),
  "tbc.": analogTape,
  "tbc.window_samples": (p) => p.tbc.enabled,
  "tbc.gate_samples": (p) => p.tbc.enabled,
  "tbc.reacquire_lines": (p) => p.tbc.enabled,
  "tbc.error_rate": (p) => p.tbc.enabled,
  "tbc.residual_samples": (p) => p.tbc.enabled,
  "tbc.bits": (p) => p.tbc.enabled,
  "tbc.regenerate_sync": (p) => p.tbc.enabled,

  // RF
  "rf.": composite,
  "rf.placement": (p) => p.rf.enabled,
  "rf.snow_ire": (p) => p.rf.enabled,
  "rf.ghost_gain": (p) => p.rf.enabled,
  "rf.ghost_delay_us": (p) => p.rf.enabled,
  "rf.interference_ire": (p) => p.rf.enabled,
  "rf.interference_mhz": (p) => p.rf.enabled,
  "rf.sound_beat": (p) => p.rf.enabled,

  // エンコーダ
  "encoder.luma_bw_mhz": composite,
  "encoder.chroma_gain": composite,
  "encoder.i_bw_mhz": (p) => composite(p) && p.standard === "ntsc",
  "encoder.q_bw_mhz": (p) => composite(p) && p.standard === "ntsc",
  "encoder.uv_bw_mhz": (p) => composite(p) && p.standard === "pal",

  // 受像機
  "decoder.": composite,
  "decoder.yc_separation": ycSeparated,
  "decoder.motion_adaptive": comb3d,
  "decoder.motion_threshold_ire": (p) => comb3d(p) && p.decoder.motion_adaptive,
  "decoder.frame_buffer_bits": comb3d,
  "decoder.pal_delay_line": (p) => p.standard === "pal",

  // キャプチャ
  "capture.": (p) => p.capture.enabled,
  "capture.bitrate_kbps": (p) => p.capture.codec,
  "capture.gop": (p) => p.capture.codec,
  "capture.motion_search": (p) => p.capture.codec,
  "capture.rate_control": (p) => p.capture.codec,

  // 音声
  "audio.": (p) => p.audio.enabled,
  "audio.low_cut_hz": (p) => p.audio.mode === "linear",
  "audio.high_cut_hz": (p) => p.audio.mode === "linear",
  "audio.hifi_buzz": (p) => p.audio.mode === "hifi",
};

/** グループ全体の条件 ("camera." など) を掛けない path (グループのオン・オフ自体のスイッチなど) */
const ignoreGroup = new Set([
  "input.enabled",
  "camera.enabled",
  "osd.enabled",
  "tape.enabled",
  "tape.format",
  "transport.tracking_error",
  "capture.enabled",
  "audio.enabled",
]);

/** 今の設定でそのコントロールが効くか */
export function isActive(p: Params, path: string): boolean {
  const dot = path.indexOf(".");
  if (dot > 0 && !ignoreGroup.has(path)) {
    const group = activeWhen[path.slice(0, dot + 1)];
    if (group && !group(p)) return false;
  }
  const own = activeWhen[path];
  return own ? own(p) : true;
}

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

// テープのフォーマットごとの値。NTSC / PAL で搬送波・低域変換周波数が違うものは方式で分ける。
// 数値は各規格の公称値 (搬送波はシンクチップ〜ホワイトピーク)。再生伝送路の帯域は画質の目安として決めた値
type FormatSpec = {
  fm: { ntsc: [number, number]; pal: [number, number] };
  colorUnder: { ntsc: number; pal: number };
  lumaBw: number;
  channel: number;
  noise: number;
  connection: "composite" | "s_video";
  extra?: DeepPartial<Params>;
};
// DV 方式のフォーマットの値。デッキのアナログ出力は S 映像が普通
// (DV はコンポジット信号を通すかどうかで FireWire / アナログ出力を選ぶので、その設定には触れない)
const digitalSpecs: Record<DigitalFormat, DeepPartial<Params>> = {
  dv: { dv: { burst_rate: 0, noise_rate: 0 } },
  // Hi8 のテープ (蒸着テープが多い) は傷・ドロップアウトが出やすい
  digital8: { dv: { burst_rate: 0.3, noise_rate: 0.5 } },
};

const formatSpecs: Record<Exclude<TapeFormat, "custom" | DigitalFormat>, FormatSpec> = {
  vhs: {
    fm: { ntsc: [3.4, 4.4], pal: [3.8, 4.8] }, colorUnder: { ntsc: 629, pal: 627 },
    lumaBw: 3.0, channel: 5.5, noise: 0.05, connection: "composite",
  },
  svhs: {
    fm: { ntsc: [5.4, 7.0], pal: [5.4, 7.0] }, colorUnder: { ntsc: 629, pal: 627 },
    lumaBw: 5.0, channel: 8.5, noise: 0.04, connection: "s_video",
  },
  beta: {
    fm: { ntsc: [3.6, 4.8], pal: [3.8, 5.2] }, colorUnder: { ntsc: 688, pal: 685 },
    lumaBw: 3.2, channel: 6.0, noise: 0.05, connection: "composite",
  },
  super_beta: {
    fm: { ntsc: [4.4, 5.6], pal: [4.4, 5.6] }, colorUnder: { ntsc: 688, pal: 685 },
    lumaBw: 3.8, channel: 7.0, noise: 0.045, connection: "composite",
  },
  video8: {
    fm: { ntsc: [4.2, 5.4], pal: [4.2, 5.4] }, colorUnder: { ntsc: 743, pal: 732 },
    lumaBw: 3.0, channel: 6.5, noise: 0.05, connection: "composite",
    // 薄いメタル (蒸着) テープはドロップアウトが多い
    extra: { tape: { dropout_rate: 0.004 } },
  },
  hi8: {
    fm: { ntsc: [5.7, 7.7], pal: [5.7, 7.7] }, colorUnder: { ntsc: 743, pal: 732 },
    lumaBw: 4.8, channel: 9.5, noise: 0.04, connection: "s_video",
    extra: { tape: { dropout_rate: 0.004 } },
  },
};

/// フォーマットに合わせる値 (カスタムは何も変えない)。
/// `full` = false なら、テレビ方式で変わる値 (搬送波周波数・低域変換周波数) だけを返す
export function formatDiff(format: TapeFormat, standard: Standard, full = true): DeepPartial<Params> {
  if (format === "custom") return { tape: { format } };
  if (isDigitalFormat(format)) {
    // テレビ方式で変わる値はない (720×480 4:1:1 / 720×576 4:2:0 は vhs-core が方式から決める)
    if (!full) return { tape: { format } };
    return { tape: { format }, output: { connection: "s_video" }, ...digitalSpecs[format] };
  }
  const f = formatSpecs[format];
  const [sync, white] = f.fm[standard];
  if (!full) return { tape: { fm_sync_mhz: sync, fm_white_mhz: white, color_under_khz: f.colorUnder[standard] } };
  return applyDiff(
    {
      tape: {
        format, luma_model: "fm", fm_sync_mhz: sync, fm_white_mhz: white, fm_channel_mhz: f.channel,
        fm_noise: f.noise, color_under_khz: f.colorUnder[standard], luma_bw_mhz: f.lumaBw,
        dropout_rate: 0.002,
      },
      output: { connection: f.connection },
    },
    f.extra ?? {},
  );
}

/// テレビ方式とテープのフォーマットに合わせて値を設定し直す。
/// `fullFormat` = false (方式だけを変えたとき) は、フォーマットの値のうち方式で変わるものだけ合わせる
export function retarget(p: Params, standard: Standard, format: TapeFormat, fullFormat = true): Params {
  return applyDiff(applyDiff(p, standardDiffs[standard]), formatDiff(format, standard, fullFormat));
}

// テレビ方式を切り替えたときに合わせる値 (その方式で一般的な値)
export const standardDiffs: Record<Standard, DeepPartial<Params>> = {
  ntsc: { standard: "ntsc", encoder: { luma_bw_mhz: 4.2 } },
  pal: { standard: "pal", encoder: { luma_bw_mhz: 5.0 }, audio: { hum_hz: 50 } },
};

// 組み込みプリセット (既定値からの差分。テレビ方式は今の方式を引き継ぐ)
export const presets: { name: string; diff: DeepPartial<Params> }[] = [
  { name: "VHS 標準 (SP)", diff: {} },
  {
    name: "90 年代の家庭用ビデオカメラ",
    diff: {
      input: {
        enabled: true, blur_px: 0.8, sharpness: 1.6, sharpness_radius_px: 1.6, sharpness_vertical: 0.4,
        sharpness_coring: 0.01, sharpness_limit: 0.18, contrast: 1.08, saturation: 1.15, color_temp: 0.12,
      },
    },
  },
  {
    name: "放送用プランビコン 3 管カメラ",
    diff: { camera: { enabled: true, act: true } },
  },
  {
    name: "家庭用ビジコンカメラ",
    diff: {
      camera: {
        enabled: true, target_gamma: 0.65, camera_gamma: 0.68, capacitive_lag: 0.35, photo_lag: 0.3, photo_lag_fields: 12,
        beam_current: 1.4, dark_current: 0.02, burn_rate: 0.15, burn_recovery: 0.03, registration_px: 1.2, mismatch: 0.45,
        resolution_px: 1.2, noise_pct: 2, amp_drive: 0.7, crosstalk: 0.06, flare: 0.04, shading: 0.15,
      },
    },
  },
  {
    name: "サチコンカメラ",
    diff: { camera: { enabled: true, target_gamma: 1.0, camera_gamma: 0.43, capacitive_lag: 0.05, photo_lag: 0.03, burn_rate: 0.04 } },
  },
  {
    name: "劣化した撮像管カメラ",
    diff: {
      camera: {
        enabled: true, target_gamma: 0.7, camera_gamma: 0.65, aging: 0.75, blemishes: 25, hysteresis: 0.03, burn_rate: 0.2,
        burn_recovery: 0.02, mismatch: 0.6, registration_px: 2.5, amp_drive: 1.2, amp_asymmetry: 0.6,
      },
    },
  },
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
    name: "TBC 付き S-VHS デッキ",
    diff: { tbc: { enabled: true } },
  },
  {
    name: "TBC が暴れる劣化テープ",
    diff: {
      tape: { luma_noise_ire: 5, dropout_rate: 0.015, dropout_length: 80 },
      transport: { jitter_samples: 2.5, head_switch_offset: 24, tracking_error: 0.45, skew_samples: 12 },
      tbc: { enabled: true, window_samples: 20, gate_samples: 4, reacquire_lines: 6, error_rate: 0.004 },
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
    name: "電波の弱い放送を録画",
    diff: {
      rf: { enabled: true, placement: "before_tape", snow_ire: 8, ghost_gain: 0.25, ghost_delay_us: 2.5 },
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
  {
    name: "MiniDV カムコーダ (FireWire 取り込み)",
    diff: { tape: { format: "dv" }, encoder: { enabled: false }, audio: { enabled: false } },
  },
  {
    name: "MiniDV をアナログ出力 (S 映像)",
    diff: { tape: { format: "dv" }, output: { connection: "s_video" }, audio: { enabled: false } },
  },
  {
    name: "劣化した MiniDV テープ",
    diff: {
      tape: { format: "dv" }, encoder: { enabled: false },
      dv: { noise_rate: 3, noise_length: 30, noise_density: 0.08, ecc_miss: 0.15, burst_rate: 0.5, head_clog_rate: 1 },
      audio: { enabled: false },
    },
  },
  {
    name: "壊れた DV ファイル (誤り訂正なし)",
    diff: {
      tape: { format: "dv" }, encoder: { enabled: false },
      dv: { noise_rate: 4, noise_length: 60, noise_density: 0.03, ecc: false, concealment: false },
      audio: { enabled: false },
    },
  },
  {
    name: "撮像管カメラだけ (信号処理なし)",
    diff: { camera: { enabled: true }, encoder: { enabled: false }, tape: { enabled: false } },
  },
  {
    name: "PAL VHS (シンプル PAL 受像機)",
    diff: { ...standardDiffs.pal, decoder: { pal_delay_line: false, burst_pll: 0.3 } },
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
