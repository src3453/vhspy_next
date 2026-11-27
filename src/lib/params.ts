// vhs-core の Params と対応する型と、GUI 用のパラメータ定義。
// Rust 側 (crates/vhs-core/src/params.rs) を変更したらここも合わせる。

export type Standard = "ntsc" | "pal";
export type TapeFormat = "vhs" | "svhs" | "beta" | "super_beta" | "video8" | "hi8" | "dv" | "digital8" | "custom";
/** DV 方式 (デジタル記録) のフォーマット */
type DigitalFormat = "dv" | "digital8";
export const isDigitalFormat = (f: TapeFormat): f is DigitalFormat => f === "dv" || f === "digital8";

export type ClockParams = {
  offset_ppm: number;
  drift_ppm: number;
  drift_time_s: number;
  wow_ppm: number;
  wow_hz: number;
  flutter_ppm: number;
  flutter_hz: number;
  jitter_ns: number;
};

export type Params = {
  /** テレビ方式 */
  standard: Standard;
  /** NTSC を 1 ライン 228 周期の非標準ラインにする (CGA・Apple II などの出力) */
  ntsc_228: boolean;
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
  encoder: { enabled: boolean; luma_bw_mhz: number; i_bw_mhz: number; q_bw_mhz: number; uv_bw_mhz: number; chroma_gain: number; burst_level_pct: number; setup: Setup; white_point: WhitePoint; gamma: number };
  signal: {
    vertical_sync: { jitter: number; attenuation_pct: number; noise_ire: number };
    horizontal_sync: { jitter: number; attenuation_pct: number; noise_ire: number };
    color_burst: { jitter: number; attenuation_pct: number; noise_ire: number };
    chroma_subcarrier: { jitter: number; attenuation_pct: number; noise_ire: number };
  };
  vbi: {
    enabled: boolean;
    closed_caption: boolean;
    caption_text: string;
    caption_mode: "pop_on" | "roll_up" | "paint_on";
    caption_channel: number;
    vitc: boolean;
    vits: boolean;
    teletext: boolean;
    teletext_system: "nabts" | "jtes";
    teletext_lines: number;
    jtes_program: number;
    jtes_header: string;
    jtes_pages: string;
    jtes_superimpose: boolean;
    wss: boolean;
  };
  tape: {
    enabled: boolean;
    format: TapeFormat;
    luma_model: "simple" | "fm";
    fm_sync_mhz: number;
    fm_white_mhz: number;
    fm_channel_mhz: number;
    fm_noise: number;
    writing_speed_mps: number;
    head_spacing_um: number;
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
    magnetic_model: "linear" | "tanh" | "jiles_atherton";
    magnetic_media: MagneticMedia;
    rf_drive_hc: number;
    saturation_ire: number;
    rf_saturation: number;
    dropout_rate: number;
    dropout_length: number;
    dropout_depth_um: number;
    doc_threshold_db: number;
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
    agc_enabled: boolean;
    agc_mode: "keyed" | "peak" | "average" | "manual";
    agc_time_ms: number;
    agc_range_db: number;
    agc_offset_db: number;
    signal_db: number;
    overload_db: number;
    flutter_db: number;
    flutter_hz: number;
  };
  decoder: {
    yc_separation: "notch" | "comb" | "comb3d";
    motion_threshold_ire: number;
    motion_adaptive: boolean;
    frame_buffer_bits: number;
    chroma_bw_mhz: number;
    luma_shelf_db: number;
    luma_shelf_mhz: number;
    chroma_shelf_db: number;
    chroma_shelf_mhz: number;
    setup: Setup;
    white_point: WhitePoint;
    gamma: number;
    chroma_demod: "iq" | "iq_wide" | "ry_by" | "xz";
    narrow_bw_mhz: number;
    xz_x_axis_deg: number;
    xz_x_gain: number;
    xz_z_axis_deg: number;
    xz_z_gain: number;
    xz_cathode_k: number;
    acc: boolean;
    color_killer: boolean;
    h_pll: number;
    burst_pll: number;
    clocks: { chroma: ClockParams; horizontal: ClockParams; vertical: ClockParams };
    pal_delay_line: boolean;
    ynr_strength: number;
    dnr_strength: number;
    dnr_motion_threshold_ire: number;
    nr_bits: number;
    contrast: number;
    brightness: number;
    saturation: number;
    hue_deg: number;
    v_hold_pct: number;
    v_sync_window: number;
    v_integrator_us: number;
    vertical_position: number;
    caption_display: boolean;
    caption_channel: number;
    teletext_display: boolean;
    teletext_program: number;
    teletext_page: number;
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
    tape_speed: number;
    head_gap_um: number;
    head_spacing_um: number;
    bias_level: number;
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
  output: {
    deinterlace: "weave" | "bob" | "blend" | "motion_adaptive_blend" | "yadif" | "qtgmc";
    deinterlace_params: {
      weave_temporal_weight_pct: number;
      bob_blend_pct: number;
      blend_strength_pct: number;
      motion_adaptive_threshold: number;
      motion_adaptive_blend_pct: number;
      yadif_motion_threshold: number;
      yadif_search_radius: number;
      yadif_temporal_weight_pct: number;
      qtgmc_block_size: number;
      qtgmc_search_radius: number;
      qtgmc_temporal_weight_pct: number;
      qtgmc_sharpness_pct: number;
    };
    connection: "composite" | "s_video";
  };
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
    font_rounding: boolean;
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
    fit: "stretch" | "original" | "cover" | "contain" | "pixel";
    zoom: number;
    pan_x: number;
    pan_y: number;
    speed_x: number;
    speed_y: number;
    exposure_ev: number;
    /** ドットクロック同期の配置で右へずらすサンプル数 (0..3) */
    pixel_phase: number;
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
      b("ntsc_228", "228 周期ライン (コンピュータ出力)"),
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
          { value: "original", label: "オリジナルサイズ (画像の画素を維持)" },
          { value: "cover", label: "画面を覆う (はみ出しはパンで見る)" },
          { value: "contain", label: "画面に収める (余白は黒)" },
          { value: "pixel", label: "ドットクロック同期 (アーティファクトカラー)" },
        ],
      },
      r("image.zoom", "拡大", 1, 4, 0.01, "×"),
      r("image.pan_x", "横位置", -1, 1, 0.01),
      r("image.pan_y", "縦位置", -1, 1, 0.01),
      r("image.speed_x", "横パン速度", -1, 1, 0.005, "画面/s"),
      r("image.speed_y", "縦パン速度", -1, 1, 0.005, "画面/s"),
      r("image.exposure_ev", "HDR 露出", -8, 8, 0.1, "EV"),
      r("image.pixel_phase", "ドット位相", 0, 3, 1, "サンプル"),
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
          { value: "direct", label: "ダイレクト (処理なし)" },
          { value: "capture", label: "キャプチャ段のみ" },
          { value: "composite", label: "デッキ出力に重畳 (コンポジット)" },
          { value: "tape", label: "テープに焼き込み (録画済み)" },
        ],
      },
      { kind: "markup", path: "osd.markup", label: "表示内容" },
      { kind: "font", path: "osd.font", label: "フォント" },
      r("osd.font_index", "TTC 内の番号", 0, 7, 1),
      r("osd.font_dots", "ドット数 (0=なめらか)", 0, 32, 1, "dot"),
      b("osd.font_rounding", "SAA5050 風の斜め補間"),
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
      r("tape.writing_speed_mps", "ヘッドの書き込み速度", 2, 10, 0.01, "m/s"),
      r("tape.head_spacing_um", "ヘッドの汚れ (スペーシング)", 0, 1, 0.01, "µm"),
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
    note: "Jiles–Atherton はテープの種類ごとに保磁力と角形比へ当てはめた係数で、記録磁界を保磁力 Hc の倍数で指定する。" +
      "FM 輝度 RF はラインごとに状態を持って解き (8 倍オーバーサンプリング)、色と音声はバイアス (色は FM 搬送波、音声は交流バイアス) を掛けた J-A から作った曲線を使う。" +
      "係数は公称の保磁力・角形比からの見積もりで、実際のテープを測った値ではない。",
    controls: [
      {
        kind: "select", path: "tape.magnetic_model", label: "磁性体モデル",
        options: [
          { value: "tanh", label: "tanh 飽和" },
          { value: "jiles_atherton", label: "Jiles–Atherton (ヒステリシス)" },
          { value: "linear", label: "線形" },
        ],
      },
      r("tape.saturation_ire", "色の飽和レベル", 5, 150, 1, "IRE"),
      r("tape.rf_saturation", "RF の飽和レベル", 0.2, 5, 0.05),
      {
        kind: "select", path: "tape.magnetic_media", label: "テープの種類",
        options: [
          { value: "cobalt", label: "コバルト酸化鉄 (VHS・ベータ)" },
          { value: "high_cobalt", label: "高保磁力コバルト (S-VHS)" },
          { value: "metal_particle", label: "メタル (8mm・Hi8)" },
          { value: "ferric_audio", label: "音声用酸化鉄 (論文の係数)" },
        ],
      },
      r("tape.rf_drive_hc", "FM の記録磁界", 0.3, 6, 0.05, "×Hc"),
    ],
  },
  {
    title: "テープ: ドロップアウト",
    controls: [
      r("tape.dropout_rate", "発生率 (/ライン)", 0, 0.1, 0.0005),
      r("tape.dropout_length", "平均長", 1, 400, 1, "smp"),
      r("tape.dropout_depth_um", "テープの浮き", 0, 3, 0.05, "µm"),
      b("tape.dropout_compensation", "ドロップアウト補償 (DOC)"),
      r("tape.doc_threshold_db", "DOC の検出レベル", 3, 40, 1, "dB"),
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
      b("rf.agc_enabled", "受信レベルと AGC を模擬"),
      {
        kind: "select", path: "rf.agc_mode", label: "AGC 方式",
        options: [
          { value: "keyed", label: "キード (同期先端)" },
          { value: "peak", label: "ピーク" },
          { value: "average", label: "平均値" },
          { value: "manual", label: "固定利得 (AGC なし)" },
        ],
      },
      r("rf.signal_db", "受信レベル", -50, 30, 0.5, "dB"),
      r("rf.agc_time_ms", "AGC 時定数", 0.1, 1000, 0.1, "ms"),
      r("rf.agc_range_db", "AGC 範囲", 0, 60, 1, "dB"),
      r("rf.agc_offset_db", "AGC 基準ずれ", -12, 12, 0.1, "dB"),
      r("rf.overload_db", "過入力レベル", 0, 30, 0.5, "dB"),
      r("rf.flutter_db", "フラッター", 0, 20, 0.1, "dB"),
      r("rf.flutter_hz", "フラッター周波数", 0.1, 30, 0.1, "Hz"),
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
      r("encoder.burst_level_pct", "バースト振幅", 0, 300, 1, "%"),
      {
        kind: "select", path: "encoder.setup", label: "セットアップ (黒レベル)",
        options: [
          { value: "auto", label: "方式の標準 (NTSC 7.5 / PAL 0 IRE)" },
          { value: "ire7_5", label: "7.5 IRE (RS-170A / SMPTE 170M)" },
          { value: "ire0", label: "0 IRE (旧 NTSC-J)" },
        ],
      },
      {
        kind: "select", path: "encoder.white_point", label: "番組の基準白",
        options: [
          { value: "d65", label: "D65 (SMPTE 170M)" },
          { value: "d93", label: "D93 (9300 K + 27 MPCD・日本)" },
        ],
      },
      r("encoder.gamma", "送り側ガンマ補正 (想定 γ)", 1, 3.5, 0.05),
    ],
  },
  {
    title: "信号劣化 (同期・色信号)",
    note: "送出側の各信号要素に個別の揺れ・レベル減衰・雑音を加える。信号は等化パルス・切り込み付き垂直同期・VBI を含む全ラスタで作り、垂直同期の減衰・雑音は幅広パルスそのものに掛かる。受像機の垂直同期分離が取り損ねると画面が流れる。",
    controls: [
      r("signal.vertical_sync.jitter", "垂直同期ジッタ", 0, 5, 0.05, "line"),
      r("signal.vertical_sync.attenuation_pct", "垂直同期レベル減衰", 0, 100, 1, "%"),
      r("signal.vertical_sync.noise_ire", "垂直同期雑音", 0, 40, 0.1, "IRE rms"),
      r("signal.horizontal_sync.jitter", "水平同期ジッタ", 0, 12, 0.1, "smp"),
      r("signal.horizontal_sync.attenuation_pct", "水平同期レベル減衰", 0, 100, 1, "%"),
      r("signal.horizontal_sync.noise_ire", "水平同期雑音", 0, 40, 0.1, "IRE rms"),
      r("signal.color_burst.jitter", "カラーバースト位相ジッタ", 0, 90, 0.5, "° rms"),
      r("signal.color_burst.attenuation_pct", "カラーバーストレベル減衰", 0, 100, 1, "%"),
      r("signal.color_burst.noise_ire", "カラーバースト雑音", 0, 20, 0.1, "IRE rms"),
      r("signal.chroma_subcarrier.jitter", "色副搬送波位相ジッタ", 0, 90, 0.5, "° rms"),
      r("signal.chroma_subcarrier.attenuation_pct", "色副搬送波レベル減衰", 0, 100, 1, "%"),
      r("signal.chroma_subcarrier.noise_ire", "色副搬送波雑音", 0, 20, 0.1, "IRE rms"),
    ],
  },
  {
    title: "VBI (帰線区間のデータ・試験信号)",
    note: "画面外 (垂直帰線区間) に重畳する信号。受像機の垂直ホールドがずれて画面が流れたとき、画面位置を上げたとき、くし形フィルタやドロップアウト補償が隣のラインを使うときに見えてくる。テープにもそのまま記録される。",
    controls: [
      b("vbi.enabled", "VBI 信号を載せる"),
      b("vbi.closed_caption", "字幕 (EIA-608、NTSC ライン 21 / 284)"),
      { kind: "text", path: "vbi.caption_text", label: "字幕の文 (\\n で改行)", placeholder: "VHS NEXT CLOSED CAPTION TEST" },
      {
        kind: "select", path: "vbi.caption_mode", label: "字幕の送り方",
        options: [
          { value: "pop_on", label: "ポップオン (1 行ずつ表示)" },
          { value: "roll_up", label: "ロールアップ (3 行で流す)" },
          { value: "paint_on", label: "ペイントオン (届いた順に描く)" },
        ],
      },
      r("vbi.caption_channel", "字幕の送出チャンネル (CC1〜CC4)", 1, 4, 1),
      b("vbi.vits", "VITS (試験信号)"),
      b("vbi.vitc", "VITC (タイムコード)"),
      b("vbi.teletext", "文字多重 (PAL WST / NTSC NABTS・JTES)"),
      {
        kind: "select", path: "vbi.teletext_system", label: "NTSC 文字多重の方式",
        options: [
          { value: "nabts", label: "NABTS (北米)" },
          { value: "jtes", label: "JTES (日本の文字放送)" },
        ],
      },
      r("vbi.teletext_lines", "文字多重のライン数", 1, 16, 1, "line"),
      r("vbi.jtes_program", "JTES: 送る番組番号", 0, 999, 1),
      { kind: "text", path: "vbi.jtes_header", label: "JTES: ヘッダ文", placeholder: "VHS NEXT 文字放送" },
      { kind: "markup", path: "vbi.jtes_pages", label: "JTES: ページ (--- の行で区切る)" },
      b("vbi.jtes_superimpose", "JTES: スーパー (映像に重ねる)"),
      b("vbi.wss", "WSS / CGMS-A (画面サイズ識別)"),
    ],
  },
  {
    title: "再生 NR (高級 VCR)",
    note: "YNR は 1H のライン相関を使う HQ 回路、3D-DNR は約 1 フレームの映像メモリを使うデジタル再生回路の近似。動き部では残像を避けるため効きを下げる。",
    controls: [
      r("decoder.ynr_strength", "YNR (ライン相関)", 0, 1, 0.01),
      r("decoder.dnr_strength", "デジタル 3D-DNR", 0, 1, 0.01),
      r("decoder.dnr_motion_threshold_ire", "DNR: 動き判定", 0.5, 20, 0.5, "IRE"),
      r("decoder.nr_bits", "NR ADC/DAC ビット数 (0 = 量子化なし)", 0, 10, 1, "bit"),
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
      r("decoder.luma_shelf_db", "輝度ハイシェルフ", -12, 12, 0.5, "dB"),
      r("decoder.luma_shelf_mhz", "輝度ハイシェルフ周波数", 0.5, 5, 0.1, "MHz"),
      r("decoder.chroma_shelf_db", "色差ハイシェルフ", -12, 12, 0.5, "dB"),
      r("decoder.chroma_shelf_mhz", "色差ハイシェルフ周波数", 0.1, 2, 0.05, "MHz"),
      {
        kind: "select", path: "decoder.setup", label: "受像機のセットアップ",
        options: [
          { value: "auto", label: "方式の標準 (NTSC 7.5 / PAL 0 IRE)" },
          { value: "ire7_5", label: "7.5 IRE (RS-170A / SMPTE 170M)" },
          { value: "ire0", label: "0 IRE (旧 NTSC-J)" },
        ],
      },
      {
        kind: "select", path: "decoder.white_point", label: "受像機の色温度",
        options: [
          { value: "d65", label: "D65 (SMPTE 170M)" },
          { value: "d93", label: "D93 (9300 K + 27 MPCD・日本)" },
        ],
      },
      r("decoder.gamma", "受像機のガンマ γ", 1, 3.5, 0.05),
      {
        kind: "select", path: "decoder.chroma_demod", label: "色復調方式",
        options: [
          { value: "iq", label: "I/Q (同帯域・理想)" },
          { value: "iq_wide", label: "I/Q (I 広帯域・Q 狭帯域)" },
          { value: "ry_by", label: "R-Y・B-Y 2 軸 (IC 時代)" },
          { value: "xz", label: "X・Z 2 軸 (真空管時代)" },
        ],
      },
      r("decoder.narrow_bw_mhz", "狭帯域側の色差帯域", 0.1, 1.5, 0.05, "MHz"),
      r("decoder.xz_x_axis_deg", "X 復調軸", 0, 180, 0.1, "°"),
      r("decoder.xz_x_gain", "X 復調利得", 0, 4, 0.01),
      r("decoder.xz_z_axis_deg", "Z 復調軸", -90, 90, 0.1, "°"),
      r("decoder.xz_z_gain", "Z 復調利得", 0, 4, 0.01),
      r("decoder.xz_cathode_k", "共通カソード結合 k", 0, 0.333, 0.001),
      b("decoder.acc", "ACC"),
      b("decoder.color_killer", "カラーキラー"),
      r("decoder.h_pll", "水平 AFC 追従", 0, 1, 0.01),
      r("decoder.burst_pll", "色副搬送波 PLL 追従", 0.02, 1, 0.01),
      b("decoder.pal_delay_line", "PAL 遅延線"),
      r("decoder.contrast", "コントラスト", 0, 2, 0.01),
      r("decoder.brightness", "明るさ", -0.5, 0.5, 0.01),
      r("decoder.saturation", "色の濃さ", 0, 3, 0.01),
      r("decoder.hue_deg", "色相", -180, 180, 1, "°"),
      r("decoder.v_hold_pct", "垂直ホールド", -30, 30, 0.1, "%"),
      r("decoder.v_sync_window", "垂直同期の引き込み範囲", 0.02, 0.9, 0.01),
      r("decoder.v_integrator_us", "垂直同期分離の積分時定数", 2, 200, 1, "µs"),
      r("decoder.vertical_position", "画面の上下位置", -30, 30, 0.5, "line"),
      b("decoder.caption_display", "字幕デコーダ (ライン 21 / 284)"),
      r("decoder.caption_channel", "字幕デコーダのチャンネル (CC1〜CC4)", 1, 4, 1),
      b("decoder.teletext_display", "文字放送受信機 (JTES)"),
      r("decoder.teletext_program", "文字放送: 番組番号", 0, 999, 1),
      r("decoder.teletext_page", "文字放送: ページ", 1, 99, 1),
    ],
  },
  {
    title: "受像機: 発振器 (クロック)",
    note: "受像機の水晶・セラミック振動子などの周波数誤差。ppm は公称周波数に対する比 (色副搬送波 NTSC で 1 ppm ≒ 3.58 Hz、水平で 1 ppm ≒ 0.016 Hz)。ずれ・ドリフト・ワウ・フラッタは周波数に、ジッタはラインごとの時間に掛かる。",
    controls: [
      r("decoder.clocks.chroma.offset_ppm", "色副搬送波 VCXO: 周波数ずれ", -300, 300, 0.5, "ppm"),
      r("decoder.clocks.chroma.drift_ppm", "色副搬送波 VCXO: ドリフト", 0, 50, 0.5, "ppm rms"),
      r("decoder.clocks.chroma.drift_time_s", "色副搬送波 VCXO: ドリフトの時間", 0.1, 600, 0.1, "s"),
      r("decoder.clocks.chroma.wow_ppm", "色副搬送波 VCXO: ワウ", 0, 50, 0.5, "ppm"),
      r("decoder.clocks.chroma.wow_hz", "色副搬送波 VCXO: ワウの周波数", 0.05, 5, 0.05, "Hz"),
      r("decoder.clocks.chroma.flutter_ppm", "色副搬送波 VCXO: フラッタ", 0, 50, 0.5, "ppm"),
      r("decoder.clocks.chroma.flutter_hz", "色副搬送波 VCXO: フラッタの周波数", 1, 120, 0.5, "Hz"),
      r("decoder.clocks.chroma.jitter_ns", "色副搬送波 VCXO: ジッタ", 0, 50, 0.5, "ns rms"),
      r("decoder.clocks.horizontal.offset_ppm", "水平発振器: 周波数ずれ", -50000, 50000, 10, "ppm"),
      r("decoder.clocks.horizontal.drift_ppm", "水平発振器: ドリフト", 0, 5000, 10, "ppm rms"),
      r("decoder.clocks.horizontal.drift_time_s", "水平発振器: ドリフトの時間", 0.1, 600, 0.1, "s"),
      r("decoder.clocks.horizontal.wow_ppm", "水平発振器: ワウ", 0, 5000, 10, "ppm"),
      r("decoder.clocks.horizontal.wow_hz", "水平発振器: ワウの周波数", 0.05, 5, 0.05, "Hz"),
      r("decoder.clocks.horizontal.flutter_ppm", "水平発振器: フラッタ", 0, 5000, 10, "ppm"),
      r("decoder.clocks.horizontal.flutter_hz", "水平発振器: フラッタの周波数", 1, 120, 0.5, "Hz"),
      r("decoder.clocks.horizontal.jitter_ns", "水平発振器: ジッタ", 0, 1000, 5, "ns rms"),
      r("decoder.clocks.vertical.offset_ppm", "垂直発振器: 周波数ずれ", -100000, 100000, 100, "ppm"),
      r("decoder.clocks.vertical.drift_ppm", "垂直発振器: ドリフト", 0, 20000, 100, "ppm rms"),
      r("decoder.clocks.vertical.drift_time_s", "垂直発振器: ドリフトの時間", 0.1, 600, 0.1, "s"),
      r("decoder.clocks.vertical.wow_ppm", "垂直発振器: ワウ", 0, 20000, 100, "ppm"),
      r("decoder.clocks.vertical.wow_hz", "垂直発振器: ワウの周波数", 0.05, 5, 0.05, "Hz"),
      r("decoder.clocks.vertical.flutter_ppm", "垂直発振器: フラッタ", 0, 20000, 100, "ppm"),
      r("decoder.clocks.vertical.flutter_hz", "垂直発振器: フラッタの周波数", 1, 120, 0.5, "Hz"),
      r("decoder.clocks.vertical.jitter_ns", "垂直発振器: ジッタ", 0, 500000, 1000, "ns rms"),
    ],
  },
  {
    title: "出力 (インターレース解除)",
    note: "方式ごとに調整項目が切り替わります。Weave と Bob はフィールド合成量、動き適応系は動き判定や補間、QTGMC は未完成の軽量実装の動き探索を調整します。",
    controls: [
      {
        kind: "select", path: "output.deinterlace", label: "方式",
        options: [
          { value: "weave", label: "Weave" },
          { value: "bob", label: "Bob" },
          { value: "blend", label: "Blending" },
          { value: "motion_adaptive_blend", label: "Motion-adaptive Blending" },
          { value: "yadif", label: "Yadif (Yadifmod2)" },
          { value: "qtgmc", label: "QTGMC (簡易版・未完成)" },
        ],
      },
      r("output.deinterlace_params.weave_temporal_weight_pct", "Weave: 前フィールド保持", 0, 100, 1, "%"),
      r("output.deinterlace_params.bob_blend_pct", "Bob: 隣接ライン補間", 0, 100, 1, "%"),
      r("output.deinterlace_params.blend_strength_pct", "Blending: 平均量", 0, 100, 1, "%"),
      r("output.deinterlace_params.motion_adaptive_threshold", "M-A Blend: 動きしきい値", 1, 64, 1, "階調"),
      r("output.deinterlace_params.motion_adaptive_blend_pct", "M-A Blend: 動き部の平均量", 0, 100, 1, "%"),
      r("output.deinterlace_params.yadif_motion_threshold", "Yadif: 動きしきい値", 1, 64, 1, "階調"),
      r("output.deinterlace_params.yadif_search_radius", "Yadif: エッジ探索半径", 1, 8, 1, "px"),
      r("output.deinterlace_params.yadif_temporal_weight_pct", "Yadif: 静止部の時間合成", 0, 100, 1, "%"),
      r("output.deinterlace_params.qtgmc_block_size", "QTGMC: 動き探索ブロック", 8, 32, 4, "px"),
      r("output.deinterlace_params.qtgmc_search_radius", "QTGMC: 動き探索半径", 0, 6, 1, "px/field line"),
      r("output.deinterlace_params.qtgmc_temporal_weight_pct", "QTGMC: 時間合成", 0, 100, 1, "%"),
      r("output.deinterlace_params.qtgmc_sharpness_pct", "QTGMC: シャープネス", 0, 50, 1, "%"),
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
      r("audio.tape_speed", "テープ速度 (SP 比)", 0.2, 1.5, 0.01),
      r("audio.head_gap_um", "音声ヘッドのギャップ", 0.2, 5, 0.05, "µm"),
      r("audio.head_spacing_um", "音声ヘッドの汚れ (スペーシング)", 0, 3, 0.01, "µm"),
      r("audio.drive_db", "録音レベル", -12, 18, 0.5, "dB"),
      r("audio.bias_level", "交流バイアス", 0, 6, 0.05, "×Hc"),
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
  "image.fit": "画像を画面 (4:3) に合わせる方法。「オリジナルサイズ」は画像の画素数を保ち、画面より大きい部分は位置・パンで見る。「ドットクロック同期」は 1 画素を 4fsc の整数サンプル × 整数ラインに補間せず置き (640 幅 → 1 サンプル、320 幅 → 2 サンプル)、横位置を副搬送波の周期に揃える。CGA・Apple II の画面のアーティファクトカラー用。",
  ntsc_228:
    "NTSC の 1 ラインを 227.5 周期ではなく 228 周期 (912 サンプル) にする。CGA・Apple II などのコンピュータの出力と同じ非標準ライン。" +
    "副搬送波の位相がラインごとに反転しないので、画素の縞によるアーティファクトカラーが全ラインで同じ色になる。" +
    "くし形・3 次元 Y/C 分離では色が消える (実機のくし形フィルタ付きテレビと同じ) のでトラップ (ノッチ) で見る。",
  "image.pixel_phase":
    "ドットクロック同期の配置で画像を右にずらすサンプル数。副搬送波との位相が 90° ずつ変わり、アーティファクトカラーの色相が変わる。",
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
    "ダイレクト: すべての映像処理の後で画面に重ねる (文字はテープや受像機の劣化を受けない)。\n" +
    "キャプチャ段のみ: アナログ映像処理後、キャプチャ処理の直前に重ねる。キャプチャ処理が有効なとき表示され、キャプチャ由来の劣化を受ける。\n" +
    "デッキ出力に重畳: テープ再生・TBC 後のコンポジット信号に同期して文字をミックスする。文字自体はテープ処理を受けず、後段の RF・復調処理を受ける。TBC に残る同期ジッターでは位置が揺れる。\n" +
    "テープに焼き込み: 録画時に記録された文字 (カメラの日付など)。テープの劣化を一緒に受ける。",
  "osd.markup": "表示内容のマークアップ。書式はグループ上部の説明を参照。",
  "osd.scale": "既定の文字サイズ。1 = 1 ドットが 2×2 画素。<size=> タグで部分的に変更できる。",
  "osd.opacity": "OSD 全体の不透明度。0 で透明、1 で不透明。",
  "osd.font":
    "表示に使うフォントファイル (.ttf / .otf / .ttc)。空なら内蔵の VCR-JP ドットフォント (半角カナ対応)。\n" +
    "例: C:\\Windows\\Fonts\\msgothic.ttc (日本語も表示できる)。フォントにない文字は内蔵フォントで代用する。",
  "osd.font_index": ".ttc (フォントコレクション) の中のどのフォントを使うか。msgothic.ttc なら 0 = MS ゴシック、2 = MS P ゴシック。",
  "osd.font_dots":
    "外部フォントを 1 文字の高さあたり何ドットで描くか。16 で漢字 ROM 風のドット文字になる。0 はなめらかに描く。内蔵 VCR-JP は 8 ドット。",
  "osd.font_rounding": "ドット表示の斜めの角を半ドットで補い、SAA5050 風に滑らかにする。なめらかな外部フォントには効かない。",
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
  "tape.chroma_delay_samples": "VCR の Y/C 遅延の整合のあとに残る、輝度に対する色の位置ずれ (1 サンプル ≒ 70 ns)。正で色が右にずれる。",
  "tape.magnetic_model":
    "磁性体の伝達特性。tanh は記録レベルが大きいと磁化が飽和する: " +
    "強い色が頭打ちになり、FM の搬送波と色の相互変調で鮮やかな色の部分に輝度のモアレが出て、" +
    "リニア音声は大きい音が潰れる (テープコンプレッション)。Jiles–Atherton は磁化の履歴 (ヒステリシス) を持つ物理モデルで、" +
    "テープの種類・FM の記録磁界・音声の交流バイアスで飽和の仕方が決まる。線形は飽和なし。",
  "tape.saturation_ire": "低域変換クロマの磁気飽和が始まる色の振幅。小さいほど鮮やかな色が早く潰れる (tanh のみ。J-A では色の記録レベルと FM の記録磁界で決まる)。",
  "tape.rf_saturation":
    "FM 輝度の RF (搬送波の振幅 = 1) の飽和レベル。小さいほど搬送波と色の相互変調が強くなり、" +
    "鮮やかな色の部分に細かいモアレが出る (tanh のみ)。",
  "tape.magnetic_media":
    "テープの磁性体 (J-A の係数)。コバルト酸化鉄は VHS・ベータの標準テープ (角形比 0.85)、高保磁力コバルトは S-VHS (0.9)、" +
    "メタルは 8mm・Hi8 (0.92)。角張ったループほど記録磁界が保磁力を超えたところで急に飽和する。音声用酸化鉄は論文の係数 (角形比 0.26 の丸いループ)。",
  "tape.rf_drive_hc":
    "FM 搬送波を記録する磁界の振幅 (保磁力 Hc の倍数)。2 で再生 RF が基準の大きさ。小さいと磁化しきれず RF が弱くノイズが増え、" +
    "大きいと飽和して搬送波と色の相互変調 (鮮やかな色の部分の輝度のモアレ) が増える。搬送波は色のバイアスでもあるので、" +
    "大きいほど色は直線的に記録される。",

  // テープ: ドロップアウト
  "tape.dropout_rate": "1 ラインあたりのドロップアウト (磁性体の欠落による白や黒の横筋) の発生確率。",
  "tape.dropout_length": "ドロップアウト 1 回の平均の長さ。",
  "tape.dropout_depth_um":
    "ドロップアウトでテープがヘッドから浮く量の目安 (傷・ゴミ・磁性体の欠け)。損失は搬送波の高い白ほど大きいので、" +
    "浅いものは白い部分だけがざらつき・黒い筋になり、深いものは RF が消えて DOC か砂嵐になる。色も浮きのぶん薄くなる。",
  "tape.dropout_compensation": "RF の落ち込みが検出レベルを超えたところを 1 ライン前の映像で埋める (DOC)。オフだと白い筋・砂嵐がそのまま出る。",
  "tape.doc_threshold_db": "DOC がドロップアウトとみなす RF レベルの低下 (中心搬送波で)。大きくすると浅いドロップアウトは補償されずにそのまま見える。",

  // テープ: 傷み
  "tape.crease_rate": "テープのしわ・折れ目が現れる頻度。画面を帯状のノイズが流れる。しわの山でテープがヘッドから浮く (スペーシング) ので、搬送波の高い白い部分から崩れる。",
  "tape.crease_fields": "しわ 1 回が画面に現れ続ける長さ (フィールド数、60 で約 1 秒)。",
  "tape.head_clog_rate": "ヘッドの目詰まりが起きる頻度。画面全体が一瞬砂嵐に埋もれる。ギャップに付いた汚れの厚みがスペーシングになり、搬送波の高いフォーマット・書き込み速度の遅いフォーマットほどひどい。",

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
  "rf.agc_enabled":
    "受信レベルの変化とチューナの AGC (自動利得制御) を模擬する。信号を負変調の包絡線に直して利得をかけ、包絡線検波してブランキングでクランプする。" +
    "スノー・ビート妨害は受信機の入口で一定なので、電波が弱いほど (AGC が利得を上げるほど) 相対的に増える。",
  "rf.agc_mode":
    "キード: 同期先端の振幅を測る。絵柄に左右されない。\n" +
    "ピーク: 包絡線の最大値を追う (速く下げ、ゆっくり戻す)。スノーのピークで利得が下がり画面が白っぽくなる。\n" +
    "平均値: 包絡線の平均を測る。負変調なので暗い場面で利得が下がってコントラストが落ち、明るい場面で上がって白飛びする。\n" +
    "固定利得: AGC なし。受信レベルの変化がそのままコントラスト・同期の深さに出る。",
  "rf.signal_db": "受信レベル (標準からの差)。下げると AGC が利得を上げ、スノー・妨害が強まる。範囲を超えるとコントラストが落ち、同期が浅くなって画面が流れる。上げすぎるとフロントエンドが飽和する。",
  "rf.agc_time_ms": "AGC が受信レベルの変化に追いつく速さ。遅いとフラッターで明るさが揺れ、速すぎる平均値 AGC は画面の上下で明るさが波打つ。",
  "rf.agc_range_db": "AGC が利得を変えられる幅 (±)。これを超える弱電界・強電界は補正しきれない。",
  "rf.agc_offset_db": "AGC の基準のずれ。+ でコントラスト過大 (白飛び・同期が深い)、- でコントラスト不足 (同期が浅く外れやすい)。",
  "rf.overload_db": "チューナの入口が歪み始める受信レベル。受信レベルがこれを超えると同期先端・黒側が潰れ、同期が不安定になる。",
  "rf.flutter_db": "飛行機の反射などで受信レベルが周期的に揺れる深さ (フラッター)。AGC が追いつかないと明るさ・スノーが脈打つ。",
  "rf.flutter_hz": "フラッターの周波数。飛行機の通過に合わせてゆっくり変わる。",

  // エンコーダ
  standard:
    "テレビ方式。NTSC は位相の誤差がそのまま色相のずれになる。PAL は V 成分をラインごとに反転して送るので、" +
    "受像機の遅延線で上下のラインを平均すると位相の誤差が打ち消され、色相ではなく彩度の低下になる。",
  "encoder.luma_bw_mhz": "送出側の輝度帯域。NTSC 放送は 4.2 MHz、PAL-B/G は 5 MHz。",
  "encoder.uv_bw_mhz": "PAL の U/V 信号の帯域。規格は 1.3 MHz。NTSC では使わない。",
  "encoder.i_bw_mhz": "I 信号 (橙〜青緑方向の色差) の帯域。NTSC 規格は 1.3 MHz。",
  "encoder.q_bw_mhz": "Q 信号 (紫〜黄緑方向の色差) の帯域。NTSC 規格は 0.6 MHz。",
  "encoder.chroma_gain": "送出時の色信号の振幅倍率。大きいとテープでの色飽和が起きやすい。",
  "encoder.burst_level_pct":
    "カラーバーストの振幅 (100% = 規格の 40 IRE p-p / 300 mV p-p)。受像機の ACC はバーストを基準に色の濃さを合わせるので、" +
    "大きくすると色が薄く、小さくすると濃くなる (ACC を切ると色の濃さは変わらない)。0 にするとカラーキラーが働けば白黒になる。" +
    "CGA などのコンピュータはバーストを色信号と同じ回路で作るので規格より大きいことがあり、アーティファクトカラーの濃さの調整に使う。",
  "signal.vertical_sync.jitter": "同期に対する映像のフィールドごとの上下の揺れ (rms)。1 ラインは表示画像の 2 行分。",
  "signal.vertical_sync.attenuation_pct": "垂直同期 (幅広パルス) の振幅を下げる。受像機の積分回路がしきい値に届かなくなると垂直発振器が自走して画面が流れる。",
  "signal.vertical_sync.noise_ire": "垂直同期 (幅広パルス) に乗る雑音。積分回路が取り損ねる・早くトリガすると画面が跳ねたり流れたりする。",
  "vbi.enabled": "垂直帰線区間に字幕・試験信号などのデータを載せる。普段は画面外だが、画面が流れる・上下位置をずらすと見える。",
  "vbi.closed_caption": "EIA-608 のクローズドキャプション。ライン 21 に 503 kHz のランインと 2 バイトのデータが載る (画面上端に白い点線として見える)。",
  "vbi.caption_text": "字幕として送る文。行ごとに 1 つの字幕になる。*斜体* _下線_ {red} {yellow} などで属性を変えられる (EIA-608 の行中コード、1 桁の空白になる)。á é ñ ç ♪ ¿ ® なども送れる。",
  "vbi.caption_mode": "ポップオンは画面外のメモリで組み立ててから一度に出す (映画)。ロールアップは下から 1 行ずつ押し上げる (生放送)。ペイントオンは届いた文字から順に描く。",
  "vbi.caption_channel": "CC1/CC2 はライン 21 (第 1 フィールド)、CC3/CC4 はライン 284 (第 2 フィールド) で送る。デコーダのチャンネルと合わないと何も出ない。",
  "vbi.vits": "白バー・2T パルス・変調パルス・変調階段波とマルチバースト (NTSC ライン 17/18、PAL 17/18)。",
  "vbi.vitc": "SMPTE 12M の垂直間隔タイムコード (NTSC ライン 14/16、PAL 19/21)。90 ビットの白黒の縞。",
  "vbi.teletext": "PAL は WST テレテキスト (6.9 Mbit/s)、NTSC は NABTS。VHS の輝度帯域では潰れて灰色の帯になる。",
  "vbi.teletext_system": "NTSC の文字多重の方式。JTES は日本の文字放送 (ITU-R System D) で、14〜16・21 H / 277〜279・284 H のうち字幕・VITC が使っていないラインに、(272,190) 符号付きの 296 ビットのデータラインを送る。PAL は常に WST。",
  "vbi.teletext_lines": "文字多重に使うライン数 (フィールドあたり)。",
  "vbi.jtes_program": "JTES で送る番組の番号 (マガジン 1)。受像機の文字放送受信機で同じ番号を選ぶと見える。番組は最初のライン対 (例: 14 H / 277 H) で 1 フィールドに 1 パケットずつ送る。",
  "vbi.jtes_header": "各ページの上端 (ヘッダ文表示領域、小形文字) に出す文。後ろに番組・ページ番号が付く。{red} などの色指定が使える。",
  "vbi.jtes_pages": "ページの本文。--- だけの行でページを分け、上から 1, 2, 3… ページになる。標準の文字で 1 行 15 字 × 8 行。\n{red} {green} {yellow} {blue} {magenta} {cyan} {white} {black} で文字色、{dim-red} などで半輝度、{bg:blue} で背景色 ({bg:none} で透明)、{small} {medium} {normal} {wide} {tall} {double} で大きさ、{flash}…{/flash} でフラッシング、{reverse}…{/reverse} で反転。漢字は JIS 第 1・第 2 水準 (1983 年版) の字だけ送れる (ない字は〓)。",
  "vbi.jtes_superimpose": "ページデータヘッダのスーパーを立てる。受信機はラスタ (青い背景) を塗らず、テレビ映像に文字を重ねる。",
  "vbi.wss": "PAL はライン 23 前半の WSS、NTSC はライン 20 の CGMS-A (IEC 61880)。",
  "decoder.v_hold_pct": "垂直発振器の自走周波数のずれ。少し負 (自走が遅い) なら垂直同期で引き込まれる。正や大きく負にすると同期が取れず、画面が上下に流れて帰線区間の黒帯が見える。",
  "decoder.v_sync_window": "自走周期の終わりから、垂直同期を受け付ける範囲の割合。広いと引き込みやすいが、雑音で早めにトリガして画面が跳ねやすい。",
  "decoder.v_integrator_us": "垂直同期分離の RC 積分の時定数。短いと水平同期・雑音でも誤トリガし、長いと弱い同期を取り損ねる。",
  "decoder.vertical_position": "画面の上下位置。負にすると画面が下がり、上に帰線区間 (VITS・字幕のデータ) が見えてくる。",
  "decoder.caption_display": "受像機の字幕デコーダ (EIA-608)。垂直同期から数えたライン 21 / 284 を読み、32 桁 × 15 行の字幕を画面に出す。パリティ誤りの文字は■、取れなかったバイトは抜ける。",
  "decoder.teletext_display": "受像機の文字放送受信機 (JTES)。14〜16・21 H / 277〜279・284 H のデータラインをスライスし、(272,190) 符号で誤り訂正、CRC で検査したデータグループからページを蓄えて、選んだページを 16 ドット漢字 ROM の字形で画面に出す。ページが届くまでは「さがしています」と出る。VHS の輝度帯域ではデータが潰れるので、テープを通すと受からない。",
  "decoder.teletext_program": "受信する番組番号。送出側の番組番号と合わないと何も受からない。",
  "decoder.teletext_page": "表示するページ。ページは数秒ごとに巡回して送られるので、切り替えてから表示まで少し待つ。",
  "decoder.caption_channel": "表示する字幕のチャンネル。CC1 が普通の字幕、CC3 は第 2 フィールドの字幕 (第 2 言語など)。",
  "signal.horizontal_sync.jitter": "ラインごとの水平同期後縁の揺れ (rms)。水平 AFC が追従しきれないと画像が左右に揺れる。",
  "signal.horizontal_sync.attenuation_pct": "水平同期パルスの振幅を下げる。弱すぎると水平 AFC が同期を見失う。",
  "signal.horizontal_sync.noise_ire": "水平同期パルスに加える雑音 (rms)。同期検出が不安定になり、水平位置が揺れる。",
  "signal.color_burst.jitter": "ラインごとのバースト位相の揺れ (rms)。受像機の色副搬送波 PLL が追従し、色相の揺れになる。",
  "signal.color_burst.attenuation_pct": "受像機が色副搬送波の位相・振幅基準に使うカラーバーストの振幅を下げる。ACC やカラーキラーの動作に影響する。",
  "signal.color_burst.noise_ire": "カラーバースト区間に加える雑音 (rms)。PLL の位相推定が不安定になる。",
  "signal.chroma_subcarrier.jitter": "映像中の色副搬送波のラインごとの位相揺れ (rms)。バーストは変えず、色相や彩度に揺れが出る。",
  "signal.chroma_subcarrier.attenuation_pct": "映像中の色副搬送波の振幅を下げる。色が薄くなる。",
  "signal.chroma_subcarrier.noise_ire": "映像中の色副搬送波に加える雑音 (rms)。色の粒状ノイズになる。",

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
  "decoder.ynr_strength":
    "高級 VHS の YNR を模した再生輝度ノイズ低減。1H 遅延線の再帰的なライン相関で平坦部のノイズを抑え、差が大きい縦エッジでは効きを弱める。",
  "decoder.dnr_strength":
    "デッキ側のデジタル 3D-DNR。約 1 フレーム前の同じフィールド位相と輝度・色差を比べ、静止部分を時間方向に平均する。動きの誤判定や強い設定では残像が出る。",
  "decoder.dnr_motion_threshold_ire":
    "DNR の動き判定しきい値。大きくすると動きを静止とみなしてノイズは減るが残像が増え、小さくすると残像は減るが動き部での NR が弱くなる。",
  "decoder.nr_bits":
    "再生 NR 回路の ADC と DAC のビット数。JVC の資料に沿った初期値は 6 bit。少ないほど量子化段差が見え、0 は量子化なし。NR を両方 0 にすると量子化も通らない。",
  "decoder.chroma_bw_mhz": "受像機で復調した後の色差の帯域。小さいほど色がにじむ。I/Q (I 広帯域) 方式では I の帯域。",
  "encoder.setup":
    "送り側の黒レベル。北米の NTSC (RS-170A / SMPTE 170M) は 7.5 IRE、法令改正前の日本の NTSC-J は 0 IRE (セットアップなし)。" +
    "白は 100 IRE のままなので、黒〜白の振幅と色信号の振幅も変わる。",
  "encoder.white_point":
    "番組を作ったモニタの基準白 (視聴者が順応している白)。北米は D65、日本は 9300 K + 27 MPCD。受像機の色温度と同じなら色かぶりは出ない。",
  "encoder.gamma":
    "送り側のガンマ補正 (逆特性) が想定する受像機のガンマ。RGB を 1/γ 乗して送る。NTSC は 2.2 (SMPTE 170M の OETF は約 1/0.45)。" +
    "入力画像は sRGB (≒ 2.2 乗) とみなすので 2.2 で素通し。撮像管カメラ経由ではカメラ側のガンマを使う。",
  "decoder.luma_shelf_db":
    "受像機の輝度経路のハイシェルフ (画質調整の輪郭補正)。Y/C 分離のあとに掛ける。正で輪郭が立ちリンギングが出て、負で甘くなる。0 で素通し。",
  "decoder.luma_shelf_mhz": "輝度ハイシェルフのコーナー周波数。これより上の帯域が持ち上がる (下がる)。",
  "decoder.chroma_shelf_db":
    "復調後の色差のハイシェルフ。正で色の輪郭が立ち (にじみが減るように見える)、負でさらににじむ。0 で素通し。",
  "decoder.chroma_shelf_mhz": "色差ハイシェルフのコーナー周波数。色差帯域より低めに置くと効きが分かりやすい。",
  "decoder.gamma":
    "受像機 (ブラウン管) のガンマ。実際の管は 2.2〜2.8 程度。送り側の想定より大きいと暗部が沈んで硬い絵に、小さいと白っぽく眠い絵になる。",
  "decoder.setup":
    "受像機が黒とみなすレベル。送り側と食い違うと、7.5 IRE の信号を 0 IRE の受像機で見れば黒が浮いて全体が白っぽく、" +
    "0 IRE の信号を 7.5 IRE の受像機で見れば暗部がつぶれて黒に沈む。",
  "decoder.white_point":
    "受像機 (ブラウン管) の色温度。D65 の番組を D93 の受像機で見ると青白く、D93 の番組を D65 の受像機で見ると黄ばんで見える。",
  "decoder.chroma_demod":
    "NTSC 受像機の色復調方式。I/Q (I 広帯域) は I を約 1.3 MHz、Q を約 0.5 MHz で通し、I を遅延線で Q に揃える本来の方式。" +
    "R-Y・B-Y 2 軸は ±0.5 MHz で色差 2 つを検波し、G-Y = −0.51(R-Y) − 0.19(B-Y) のマトリクスで作る、市販機の大半の方式。" +
    "X・Z 2 軸は真空管時代の方式で、X・Z 復調出力を R-Y・B-Y 増幅管に入れ、3 本の色差増幅管の共通カソード電圧だけで G-Y 管を動かす。" +
    "増幅率がそろった 3 管ではどこかに誤差が残り、初期値では G-Y が約 −23° ずれる (緑〜マゼンタの色相・濃さが変わる)。",
  "decoder.narrow_bw_mhz": "Q・R-Y/B-Y・X/Z を通す狭帯域側の色差帯域。実機は約 0.5 MHz。",
  "decoder.xz_x_axis_deg": "X 復調軸の位相。B-Y 軸を 0°、R-Y 軸を 90°、I 軸を 123° として、色差増幅管で反転した後の向きで表す。",
  "decoder.xz_x_gain": "X 復調出力の利得。I/Q を検波した出力に対する倍率。",
  "decoder.xz_z_axis_deg": "Z 復調軸の位相 (Q 軸が 33°)。",
  "decoder.xz_z_gain": "Z 復調出力の利得。",
  "decoder.xz_cathode_k":
    "3 本の色差増幅管の共通カソード抵抗による結合。E_K = k(E_X + E_Z) で、k = gm·Rk / (1 + 3·gm·Rk)。" +
    "1/3 に近づく (Rk が大きい) ほど G-Y 管の出力は大きくなり、R-Y・B-Y から引かれる分も増える。",
  "decoder.burst_pll":
    "色副搬送波の PLL がバーストに追従する速さ (ライン単位)。1 = ラインごとに即時同期。" +
    "小さくするとテープのライン単位の位相ゆらぎを取り切れず、NTSC では色相の横縞、PAL では彩度のむらになる。" +
    "実機の APC は 0.1〜0.3 程度で、発振器の周波数ずれが定常的な色相ずれとして残る。",
  "decoder.clocks.chroma.offset_ppm":
    "受像機の色副搬送波発振器 (水晶 VCXO) の自走周波数のずれ。PLL の追従が 1 未満だと定常位相誤差が残り、" +
    "NTSC では画面全体の色相ずれ (色合いツマミで直す類のもの)、PAL では彩度低下になる。0 で理想的な発振器。14 ppm ≒ 50 Hz (NTSC)。",
  "decoder.clocks.chroma.drift_ppm": "色副搬送波発振器の温度などによるゆっくりした不規則な周波数変化 (rms)。PLL の追従が有限だと色相がゆっくりさまよう。",
  "decoder.clocks.chroma.wow_ppm": "色副搬送波発振器の遅い周期的な周波数の揺れ。色相がゆっくり行き来する。",
  "decoder.clocks.chroma.flutter_ppm": "色副搬送波発振器の速い周期的な周波数の揺れ (電源リプル・振動など)。色相が細かく揺れ、周波数によっては横縞になる。",
  "decoder.clocks.chroma.jitter_ns": "色副搬送波の基準位相のラインごとの揺れ (rms)。1 ns ≒ 1.3° (NTSC)。色相のライン単位のざらつきになる。",
  "decoder.clocks.horizontal.offset_ppm":
    "水平発振器の自走周波数のずれ。水平 AFC の追従が有限だと画面全体が横にずれ (≒ ずれ × 1 ライン × (1 − 追従) / 追従)、" +
    "引き込み範囲を超えると同期が外れて画面が斜めに裂ける。",
  "decoder.clocks.horizontal.drift_ppm": "水平発振器のゆっくりした不規則な周波数変化 (rms)。画面の横位置がゆっくりさまよう。",
  "decoder.clocks.horizontal.wow_ppm": "水平発振器の遅い周期的な周波数の揺れ。画面がゆっくり左右に揺れる。",
  "decoder.clocks.horizontal.flutter_ppm": "水平発振器の速い周期的な周波数の揺れ (電源リプルなど)。縦の線が波打つ。",
  "decoder.clocks.horizontal.jitter_ns": "水平発振器のラインごとの時間の揺れ (rms)。1 サンプル ≒ 70 ns (NTSC)。縦の輪郭がぎざぎざになる。",
  "decoder.clocks.vertical.offset_ppm":
    "垂直発振器の周波数ずれ (垂直ホールドの自走周期にさらに掛かる)。同期の窓の中なら引き込まれ、外れると画面が上下に流れる。",
  "decoder.clocks.vertical.drift_ppm": "垂直発振器のゆっくりした不規則な周波数変化 (rms)。窓の端にいると、ときどき同期が外れて流れる。",
  "decoder.clocks.vertical.wow_ppm": "垂直発振器の遅い周期的な周波数の揺れ。",
  "decoder.clocks.vertical.flutter_ppm": "垂直発振器の速い周期的な周波数の揺れ。",
  "decoder.clocks.vertical.jitter_ns": "垂直発振器が自走するときのトリガ時刻の揺れ (rms)。1 ライン ≒ 63.6 µs。自走で流れているときに画面が上下に震える。",
  "decoder.clocks.chroma.drift_time_s": "ドリフトの相関時間。これより長い時間をかけて別の周波数へ移っていく。",
  "decoder.clocks.horizontal.drift_time_s": "ドリフトの相関時間。これより長い時間をかけて別の周波数へ移っていく。",
  "decoder.clocks.vertical.drift_time_s": "ドリフトの相関時間。これより長い時間をかけて別の周波数へ移っていく。",
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
  "tape.writing_speed_mps":
    "ビデオヘッドとテープの相対速度。フォーマットとテレビ方式で決まる (VHS NTSC ≒ 5.8 m/s、PAL ≒ 4.85、ベータ ≒ 6.9 / 5.8、8mm ≒ 3.75 / 3.1)。" +
    "記録波長は 速度 / 周波数 なので、遅いほど同じ汚れでも損失が大きい。",
  "tape.head_spacing_um":
    "再生 EQ が想定していない余分なヘッド・テープ間隔 (ヘッドの汚れ、テープの浮き・摩耗)。損失は 54.6 × 間隔 / 波長 [dB] で、" +
    "搬送波の高い白ほど大きく落ちる (VHS で 0.5 µm だとホワイトで約 21 dB)。ノイズは減らないので S/N が下がり、" +
    "白い部分から先にノイズや砂嵐が出る。色 (低域変換) のノイズも少し増える。",
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
  "output.deinterlace":
    "Weave は前フィールドと交互に織り込む。Bob は現フィールドのラインを複製。Blending は隣接ラインの平均。Motion-adaptive Blending は静止部を織り込み、動き部を平均する。Yadif (Yadifmod2) はフィールド単位の静止判定とエッジ方向補間を使うネイティブ実装。QTGMC は未完成の軽量な試作実装。ブロック動き補償とエッジ方向補間のみで、標準QTGMCの全工程、高精度の多段動き推定、時間ノイズ除去、ソースマッチングは未実装。",
  "output.deinterlace_params.weave_temporal_weight_pct":
    "前フィールドを保持する割合。100% で通常の Weave。下げると前フィールドと現フィールド由来の空間補間を混ぜ、櫛状ノイズを弱めます。",
  "output.deinterlace_params.bob_blend_pct":
    "Bob の欠落ラインを、現フィールドの最近傍ラインから上下ラインの平均へ近づける量。0% はライン複製、100% は線形補間です。",
  "output.deinterlace_params.blend_strength_pct":
    "Blending の補間量。0% は Bob と同じ最近傍ライン、100% は隣接する現フィールドラインの平均です。",
  "output.deinterlace_params.motion_adaptive_threshold":
    "同じ位置の2フィールド前との差がこの値を超えると動きと判定します。小さいほど動き判定が増え、残像は減りますが空間補間が増えます。",
  "output.deinterlace_params.motion_adaptive_blend_pct":
    "動きと判定した部分の空間補間量。0% は最近傍ライン、100% は上下ラインの平均です。静止部では前フィールドを使います。",
  "output.deinterlace_params.yadif_motion_threshold":
    "同じ位置の2フィールド前との差がこの値以下なら静止とみなします。小さくすると時間残像を抑え、大きくすると前フィールドを使う領域が増えます。",
  "output.deinterlace_params.yadif_search_radius":
    "欠落ラインを補うとき、上下ラインの対応を探す水平方向の範囲。大きくすると斜めの輪郭に合わせやすくなります。",
  "output.deinterlace_params.yadif_temporal_weight_pct":
    "静止と判定した部分における前フィールドの混合量。0% は空間補間のみ、100% は前フィールドをそのまま使います。",
  "output.deinterlace_params.qtgmc_block_size":
    "QTGMC風の動き推定に使うブロック幅。高さは幅の半分です。大きいと滑らかな領域に強く、小さいと局所的な動きに追従しやすくなります。",
  "output.deinterlace_params.qtgmc_search_radius":
    "ブロック動き推定の探索範囲。水平方向は画素、垂直方向はフィールドライン単位です。大きいほど大きな移動を追えますが処理が重くなります。",
  "output.deinterlace_params.qtgmc_temporal_weight_pct":
    "動き補償した前フィールドの混合量。下げると時間残像を抑え、上げると静止部のちらつきやノイズを減らしやすくなります。",
  "output.deinterlace_params.qtgmc_sharpness_pct":
    "エッジ方向補間の後に加える輪郭強調。大きくすると細部が立ちますが、リンギングやノイズも目立ちます。",

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
  "audio.high_cut_hz": "リニアトラックの録音・再生アンプの高域カットオフ。テープ速度とヘッドによる損失はこれとは別に掛かる。",
  "audio.tape_speed":
    "リニアトラックのテープ速度 (SP に対する比。標準 1、3倍モード (EP) 0.333、LP 0.5)。SP は NTSC 33.35 mm/s、PAL 23.39 mm/s。" +
    "再生 EQ は SP 用のままなので、遅くすると波長が短くなってスペーシング損失とギャップ損失で高域がこもる。",
  "audio.head_gap_um": "音声ヘッドのギャップ長。波長がギャップと等しくなる周波数 (速度 / ギャップ、3倍モードで 1 µm なら約 11 kHz) で出力が消える。SP 標準の状態では影響しない。",
  "audio.head_spacing_um": "音声ヘッドとテープの余分な間隔 (ヘッドの汚れ、テープの摩耗)。高域ほど指数的に落ちる。",
  "audio.drive_db": "録音レベル。上げるとテープ飽和で歪む。",
  "audio.hiss_db": "テープヒス (サーッというノイズ) の量。",
  "audio.wow_pct": "ゆっくりした速度変動による音程の揺れ。",
  "audio.flutter_pct": "速い速度変動による音のざらつき・震え。",
  "audio.dropout_rate": "音が一瞬こもって痩せるドロップアウトの頻度 (回/秒)。",
  "audio.bias_level":
    "リニアトラックの交流バイアスの磁界 (保磁力 Hc の倍数、磁性体モデルが J-A のとき)。2 前後でほぼ直線に録音できる。" +
    "足りないと小さい音がデッドゾーンで歪み (クロスオーバー歪み)、大きい音も早く潰れる。多すぎると感度が落ちる (録音レベルで補われる)。",
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
const nonlinearMag = (p: Params) => p.tape.magnetic_model !== "linear";
/** S 映像が実際に使われるか (テープの再生出力を受像機へ直接つなぐときだけ) */
const sVideo = (p: Params) =>
  composite(p) &&
  p.output.connection === "s_video" &&
  p.tape.enabled &&
  !(p.rf.enabled && (p.rf.placement === "after_tape" || p.rf.placement === "both"));
const ycSeparated = (p: Params) => composite(p) && !sVideo(p);
const xzDemod = (p: Params) => p.standard === "ntsc" && p.decoder.chroma_demod === "xz";
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
  // 簡易モデルでも非線形磁性体なら、搬送波と色の相互変調の計算に搬送波・低域変換の値を使う
  "tape.fm_sync_mhz": (p) => fmModel(p) || nonlinearMag(p),
  "tape.fm_white_mhz": (p) => fmModel(p) || nonlinearMag(p),
  "tape.color_under_khz": (p) => fmModel(p) || nonlinearMag(p),
  "tape.color_under_level": (p) => fmModel(p) || nonlinearMag(p),
  "tape.fm_channel_mhz": fmModel,
  "tape.fm_noise": fmModel,
  // FM を計算するモデルではノイズ・ベアディングは伝送路から自然に出る
  "tape.luma_noise_ire": (p) => !fmModel(p),
  "tape.bearding_threshold_ire": (p) => !fmModel(p),
  "tape.bearding_strength": (p) => !fmModel(p),
  "tape.saturation_ire": (p) => p.tape.magnetic_model === "tanh",
  "tape.rf_saturation": (p) => p.tape.magnetic_model === "tanh",
  "tape.magnetic_media": (p) => p.tape.magnetic_model === "jiles_atherton",
  "tape.rf_drive_hc": (p) => p.tape.magnetic_model === "jiles_atherton",
  "audio.bias_level": (p) => p.audio.mode === "linear" && p.tape.magnetic_model === "jiles_atherton",
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
  "rf.agc_enabled": (p) => p.rf.enabled,
  "rf.agc_mode": (p) => p.rf.enabled && p.rf.agc_enabled,
  "rf.signal_db": (p) => p.rf.enabled && p.rf.agc_enabled,
  "rf.agc_time_ms": (p) => p.rf.enabled && p.rf.agc_enabled && p.rf.agc_mode !== "manual",
  "rf.agc_range_db": (p) => p.rf.enabled && p.rf.agc_enabled && p.rf.agc_mode !== "manual",
  "rf.agc_offset_db": (p) => p.rf.enabled && p.rf.agc_enabled,
  "rf.overload_db": (p) => p.rf.enabled && p.rf.agc_enabled,
  "rf.flutter_db": (p) => p.rf.enabled && p.rf.agc_enabled,
  "rf.flutter_hz": (p) => p.rf.enabled && p.rf.agc_enabled && p.rf.flutter_db > 0,

  // エンコーダ
  "signal.": composite,
  "encoder.luma_bw_mhz": composite,
  "encoder.chroma_gain": composite,
  "encoder.burst_level_pct": composite,
  "encoder.setup": composite,
  "encoder.white_point": composite,
  "encoder.gamma": composite,
  "encoder.i_bw_mhz": (p) => composite(p) && p.standard === "ntsc",
  "encoder.q_bw_mhz": (p) => composite(p) && p.standard === "ntsc",
  "encoder.uv_bw_mhz": (p) => composite(p) && p.standard === "pal",

  // 受像機
  "decoder.": composite,
  "decoder.yc_separation": ycSeparated,
  "decoder.motion_adaptive": comb3d,
  "decoder.motion_threshold_ire": (p) => comb3d(p) && p.decoder.motion_adaptive,
  "decoder.frame_buffer_bits": comb3d,
  "decoder.ynr_strength": analogTape,
  "decoder.dnr_strength": analogTape,
  "decoder.dnr_motion_threshold_ire": (p) => analogTape(p) && p.decoder.dnr_strength > 0,
  "decoder.nr_bits": analogTape,
  "decoder.pal_delay_line": (p) => p.standard === "pal",
  "decoder.luma_shelf_mhz": (p) => p.decoder.luma_shelf_db !== 0,
  "decoder.chroma_shelf_mhz": (p) => p.decoder.chroma_shelf_db !== 0,
  "decoder.chroma_demod": (p) => p.standard === "ntsc",
  "decoder.narrow_bw_mhz": (p) => p.standard === "ntsc" && p.decoder.chroma_demod !== "iq",
  "decoder.xz_x_axis_deg": xzDemod,
  "decoder.xz_x_gain": xzDemod,
  "decoder.xz_z_axis_deg": xzDemod,
  "decoder.xz_z_gain": xzDemod,
  "decoder.xz_cathode_k": xzDemod,
  ntsc_228: (p) => p.standard === "ntsc",
  "image.pixel_phase": (p) => p.image.fit === "pixel",
  "output.deinterlace_params.weave_temporal_weight_pct": (p) => p.output.deinterlace === "weave",
  "output.deinterlace_params.bob_blend_pct": (p) => p.output.deinterlace === "bob",
  "output.deinterlace_params.blend_strength_pct": (p) => p.output.deinterlace === "blend",
  "output.deinterlace_params.motion_adaptive_threshold": (p) => p.output.deinterlace === "motion_adaptive_blend",
  "output.deinterlace_params.motion_adaptive_blend_pct": (p) => p.output.deinterlace === "motion_adaptive_blend",
  "output.deinterlace_params.yadif_motion_threshold": (p) => p.output.deinterlace === "yadif",
  "output.deinterlace_params.yadif_search_radius": (p) => p.output.deinterlace === "yadif",
  "output.deinterlace_params.yadif_temporal_weight_pct": (p) => p.output.deinterlace === "yadif",
  "output.deinterlace_params.qtgmc_block_size": (p) => p.output.deinterlace === "qtgmc",
  "output.deinterlace_params.qtgmc_search_radius": (p) => p.output.deinterlace === "qtgmc",
  "output.deinterlace_params.qtgmc_temporal_weight_pct": (p) => p.output.deinterlace === "qtgmc",
  "output.deinterlace_params.qtgmc_sharpness_pct": (p) => p.output.deinterlace === "qtgmc",

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
  "audio.tape_speed": (p) => p.audio.mode === "linear",
  "audio.head_gap_um": (p) => p.audio.mode === "linear",
  "audio.head_spacing_um": (p) => p.audio.mode === "linear",
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

export type Setup = "auto" | "ire7_5" | "ire0";
export type WhitePoint = "d65" | "d93";
export type MagneticMedia = "cobalt" | "high_cobalt" | "metal_particle" | "ferric_audio";
type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K] };

// テープのフォーマットごとの値。NTSC / PAL で搬送波・低域変換周波数が違うものは方式で分ける。
// 数値は各規格の公称値 (搬送波はシンクチップ〜ホワイトピーク)。再生伝送路の帯域は画質の目安として決めた値
type FormatSpec = {
  fm: { ntsc: [number, number]; pal: [number, number] };
  colorUnder: { ntsc: number; pal: number };
  lumaBw: number;
  channel: number;
  /// ビデオヘッドの書き込み速度 [m/s] (ドラム径と回転数で決まる)
  speed: { ntsc: number; pal: number };
  /// 標準のテープの磁性体
  media: MagneticMedia;
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
    lumaBw: 3.0, channel: 5.5, noise: 0.05, connection: "composite", speed: { ntsc: 5.8, pal: 4.85 }, media: "cobalt",
  },
  svhs: {
    fm: { ntsc: [5.4, 7.0], pal: [5.4, 7.0] }, colorUnder: { ntsc: 629, pal: 627 },
    lumaBw: 5.0, channel: 8.5, noise: 0.04, connection: "s_video", speed: { ntsc: 5.8, pal: 4.85 }, media: "high_cobalt",
  },
  beta: {
    fm: { ntsc: [3.6, 4.8], pal: [3.8, 5.2] }, colorUnder: { ntsc: 688, pal: 685 },
    lumaBw: 3.2, channel: 6.0, noise: 0.05, connection: "composite", speed: { ntsc: 6.9, pal: 5.83 }, media: "cobalt",
  },
  super_beta: {
    fm: { ntsc: [4.4, 5.6], pal: [4.4, 5.6] }, colorUnder: { ntsc: 688, pal: 685 },
    lumaBw: 3.8, channel: 7.0, noise: 0.045, connection: "composite", speed: { ntsc: 6.9, pal: 5.83 }, media: "cobalt",
  },
  video8: {
    fm: { ntsc: [4.2, 5.4], pal: [4.2, 5.4] }, colorUnder: { ntsc: 743, pal: 732 },
    lumaBw: 3.0, channel: 6.5, noise: 0.05, connection: "composite", speed: { ntsc: 3.75, pal: 3.12 }, media: "metal_particle",
    // 薄いメタル (蒸着) テープはドロップアウトが多い
    extra: { tape: { dropout_rate: 0.004 } },
  },
  hi8: {
    fm: { ntsc: [5.7, 7.7], pal: [5.7, 7.7] }, colorUnder: { ntsc: 743, pal: 732 },
    lumaBw: 4.8, channel: 9.5, noise: 0.04, connection: "s_video", speed: { ntsc: 3.75, pal: 3.12 }, media: "metal_particle",
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
  if (!full) return { tape: { fm_sync_mhz: sync, fm_white_mhz: white, color_under_khz: f.colorUnder[standard], writing_speed_mps: f.speed[standard] } };
  return applyDiff(
    {
      tape: {
        format, luma_model: "fm", fm_sync_mhz: sync, fm_white_mhz: white, fm_channel_mhz: f.channel,
        fm_noise: f.noise, color_under_khz: f.colorUnder[standard], luma_bw_mhz: f.lumaBw,
        writing_speed_mps: f.speed[standard], magnetic_media: f.media,
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

// 組み込みプリセット (既定値からの差分。テレビ方式・テープのフォーマットも指定がなければ既定値)
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
      audio: { tape_speed: 0.333, hiss_db: -46, wow_pct: 0.2, flutter_pct: 0.12 },
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
    name: "飛行機フラッターの放送",
    diff: {
      rf: {
        enabled: true, placement: "before_tape", snow_ire: 2, ghost_gain: 0.1,
        agc_enabled: true, agc_mode: "keyed", agc_time_ms: 150, signal_db: -14, flutter_db: 9, flutter_hz: 4,
      },
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
