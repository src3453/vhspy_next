// イマーシブモードの OSD (デッキのメニュー表示) で使う文字とラベル。
// メニューは内蔵の VCR-JP フォント (ASCII・半角カナ・ブロック要素・半角矢印・一部の漢字) で
// 映像信号に重畳するので、表示する文字はすべてこのフォントにある文字にする。

const EXTRA =
  "¥×÷‘’‚‛“”„‟•‣▀▁▂▃▄▅▆▇█▉▊▋▌▍▎▏▐░▒▓▔▕▖▗▘▙▚▛▜▝▞▟万円千土年日月木水火金｡｢｣､･" +
  "ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝﾞﾟ￩￪￫￬￭￮";
const SUPPORTED = new Set<string>([...EXTRA]);
for (let c = 0x20; c <= 0x7e; c++) SUPPORTED.add(String.fromCharCode(c));

/** 半角矢印・記号 */
export const ARROW = { left: "￩", up: "￪", right: "￫", down: "￬", square: "￭", circle: "￮" };

// 全角カタカナ → 半角カタカナ
const FULL = "ァアィイゥウェエォオカガキギクグケゲコゴサザシジスズセゼソゾタダチヂッツヅテデトドナニヌネノハバパヒビピフブプヘベペホボポマミムメモャヤュユョヨラリルレロヮワヰヱヲンヴー・。、「」";
const HALF = [
  "ｧ", "ｱ", "ｨ", "ｲ", "ｩ", "ｳ", "ｪ", "ｴ", "ｫ", "ｵ", "ｶ", "ｶﾞ", "ｷ", "ｷﾞ", "ｸ", "ｸﾞ", "ｹ", "ｹﾞ", "ｺ", "ｺﾞ",
  "ｻ", "ｻﾞ", "ｼ", "ｼﾞ", "ｽ", "ｽﾞ", "ｾ", "ｾﾞ", "ｿ", "ｿﾞ", "ﾀ", "ﾀﾞ", "ﾁ", "ﾁﾞ", "ｯ", "ﾂ", "ﾂﾞ", "ﾃ", "ﾃﾞ", "ﾄ", "ﾄﾞ",
  "ﾅ", "ﾆ", "ﾇ", "ﾈ", "ﾉ", "ﾊ", "ﾊﾞ", "ﾊﾟ", "ﾋ", "ﾋﾞ", "ﾋﾟ", "ﾌ", "ﾌﾞ", "ﾌﾟ", "ﾍ", "ﾍﾞ", "ﾍﾟ", "ﾎ", "ﾎﾞ", "ﾎﾟ",
  "ﾏ", "ﾐ", "ﾑ", "ﾒ", "ﾓ", "ｬ", "ﾔ", "ｭ", "ﾕ", "ｮ", "ﾖ", "ﾗ", "ﾘ", "ﾙ", "ﾚ", "ﾛ", "ﾜ", "ﾜ", "ｲ", "ｴ", "ｦ", "ﾝ", "ｳﾞ",
  "ｰ", "･", "｡", "､", "｢", "｣",
];
const SYMBOLS: Record<string, string> = {
  "→": "￫", "←": "￩", "↑": "￪", "↓": "￬", "▶": "￫", "◀": "￩", "▲": "￪", "▼": "￬", "■": "￭", "●": "￮",
  "↔": "-", "°": "ﾟ", "µ": "u", "〜": "~", "～": "~", "　": " ",
};

/** 1 文字を OSD フォントの文字に直す (直せなければ "?") */
function convChar(ch: string): string {
  if (SUPPORTED.has(ch)) return ch;
  const code = ch.codePointAt(0)!;
  // ひらがな → カタカナ
  const kata = code >= 0x3041 && code <= 0x3096 ? String.fromCodePoint(code + 0x60) : ch;
  const i = FULL.indexOf(kata);
  if (i >= 0) return HALF[i];
  // 全角英数・記号 → 半角
  if (code >= 0xff01 && code <= 0xff5e) return String.fromCharCode(code - 0xfee0);
  return SYMBOLS[ch] ?? "?";
}

/** 文字列を OSD フォントで表示できる文字に直す */
export function toOsd(s: string): string {
  return [...s].map(convChar).join("");
}

/** そのまま (変換なしで) 表示できるか */
export function osdSafe(s: string): boolean {
  return [...s].every((c) => SUPPORTED.has(c) || convChar(c) !== "?");
}

/** マークアップとして解釈されないよう記号をエスケープする */
export function escapeMarkup(s: string): string {
  return s.replace(/[\\<{]/g, (c) => "\\" + c);
}

/** 表示幅 (半角 1、それ以外 = 漢字は 1 文字ぶん。フォントは等幅) */
export function osdLen(s: string): number {
  return [...s].length;
}

export function padEnd(s: string, n: number): string {
  const l = osdLen(s);
  return l >= n ? [...s].slice(0, n).join("") : s + " ".repeat(n - l);
}

export function padStart(s: string, n: number): string {
  const l = osdLen(s);
  return l >= n ? [...s].slice(l - n).join("") : " ".repeat(n - l) + s;
}

/** 単位を OSD 用に */
export function osdUnit(u?: string): string {
  if (!u) return "";
  const map: Record<string, string> = {
    "°": "ﾟ", "µs": "us", "回/s": "/s", "回/分": "/min", "画面/s": "/s", "×白": "×W", "line/f": "L/f",
  };
  return map[u] ?? toOsd(u);
}

/** グループ見出し (params.ts の groups の title と同じ並び) */
export const groupLabels: Record<string, string> = {
  "テレビ方式": "ﾃﾚﾋﾞ ﾎｳｼｷ",
  "画像入力 (静止画)": "ｾｲｼｶﾞ ﾆｭｳﾘｮｸ",
  "入力の画質調整": "ﾆｭｳﾘｮｸ ｶﾞｼﾂ",
  "撮像管カメラ: 光学系": "ｶﾒﾗ: ｺｳｶﾞｸｹｲ",
  "撮像管カメラ: ターゲット・ビーム": "ｶﾒﾗ: ﾀｰｹﾞｯﾄ/ﾋﾞｰﾑ",
  "撮像管カメラ: 増幅・プロセス": "ｶﾒﾗ: ﾌﾟﾛｾｽ",
  "OSD (画面表示)": "OSD ﾋｮｳｼﾞ",
  "テープ: フォーマット": "ﾃｰﾌﾟ: ﾌｫｰﾏｯﾄ",
  "DV (MiniDV / Digital8)": "DV",
  "テープ: 輝度 (FM 記録)": "ﾃｰﾌﾟ: ｷﾄﾞ (FM)",
  "テープ: 色 (低域変換記録)": "ﾃｰﾌﾟ: ｲﾛ",
  "テープ: 磁性体": "ﾃｰﾌﾟ: ｼﾞｾｲﾀｲ",
  "テープ: ドロップアウト": "ﾃｰﾌﾟ: ﾄﾞﾛｯﾌﾟｱｳﾄ",
  "テープ: 傷み": "ﾃｰﾌﾟ: ｲﾀﾐ",
  "走行系・ヘッド": "ｿｳｺｳｹｲ/ﾍｯﾄﾞ",
  "TBC (タイムベースコレクタ)": "TBC",
  "RF 接続 (アンテナ端子)": "RF ｾﾂｿﾞｸ",
  "エンコーダ (送出側)": "ｴﾝｺｰﾀﾞ",
  "信号劣化 (同期・色信号)": "ｼﾝｺﾞｳ ﾄﾞｳｷ/ｲﾛ",
  "受像機 (デコーダ)": "ｼﾞｭｿﾞｳｷ",
  "出力 (インターレース解除)": "ｼｭﾂﾘｮｸ ｲﾝﾀｰﾚｰｽ",
  "キャプチャ (デジタル化)": "ｷｬﾌﾟﾁｬ",
  "音声 (書き出し時)": "ｵﾝｾｲ",
};

/** コントロールのラベル (path → 18 文字以内) */
export const controlLabels: Record<string, string> = {
  standard: "ﾎｳｼｷ",
  ntsc_228: "228 ｼｭｳｷ ﾗｲﾝ",
  "image.fit": "ﾊｲﾁ",
  "image.zoom": "ｶｸﾀﾞｲ",
  "image.pan_x": "ﾖｺ ｲﾁ",
  "image.pan_y": "ﾀﾃ ｲﾁ",
  "image.speed_x": "ﾖｺ ﾊﾟﾝ",
  "image.speed_y": "ﾀﾃ ﾊﾟﾝ",
  "image.exposure_ev": "HDR ﾛｼｭﾂ",
  "image.pixel_phase": "ﾄﾞｯﾄ ｲｿｳ",

  "input.enabled": "ｶﾞｼﾂﾁｮｳｾｲ",
  "input.blur_px": "ﾎﾞｹ",
  "input.sharpness": "ｼｬｰﾌﾟﾈｽ",
  "input.sharpness_radius_px": "ﾘﾝｶｸ ﾉ ﾌﾄｻ",
  "input.sharpness_vertical": "ﾘﾝｶｸ ﾀﾃ/ﾖｺ",
  "input.sharpness_coring": "ｺｱﾘﾝｸﾞ",
  "input.sharpness_limit": "ﾘﾝｶｸ ｼﾞｮｳｹﾞﾝ",
  "input.brightness": "ｱｶﾙｻ",
  "input.contrast": "ｺﾝﾄﾗｽﾄ",
  "input.gamma": "ｶﾞﾝﾏ",
  "input.saturation": "ｻｲﾄﾞ",
  "input.hue_deg": "ｼｷｿｳ",
  "input.color_temp": "ｲﾛｵﾝﾄﾞ",
  "input.tint": "ｲﾛｶﾌﾞﾘ",

  "camera.enabled": "ｻﾂｿﾞｳｶﾝ ｶﾒﾗ",
  "camera.exposure_ev": "ﾛｼｭﾂ",
  "camera.crosstalk": "ﾌﾟﾘｽﾞﾑ ｺﾝｼｮｸ",
  "camera.flare": "ﾌﾚｱ",
  "camera.registration_px": "ﾚｼﾞｽﾄﾚｰｼｮﾝ",
  "camera.shading": "ｼｪｰﾃﾞｨﾝｸﾞ",
  "camera.resolution_px": "ﾋﾞｰﾑ ﾎﾞｹ",
  "camera.target_gamma": "ﾀｰｹﾞｯﾄ ｶﾞﾝﾏ",
  "camera.dark_current": "ｱﾝﾃﾞﾝﾘｭｳ",
  "camera.capacitive_lag": "ﾖｳﾘｮｳｾｲ ｻﾞﾝｿﾞｳ",
  "camera.photo_lag": "ｺｳﾄﾞｳﾃﾞﾝ ｻﾞﾝｿﾞｳ",
  "camera.photo_lag_fields": "ｻﾞﾝｿﾞｳ ｼﾞﾃｲｽｳ",
  "camera.bias_light": "ﾊﾞｲｱｽｺｳ",
  "camera.beam_current": "ﾋﾞｰﾑ ﾃﾞﾝﾘｭｳ",
  "camera.target_saturation": "ﾀｰｹﾞｯﾄ ﾎｳﾜ",
  "camera.act": "ACT",
  "camera.act_level": "ACT ﾚﾍﾞﾙ",
  "camera.hysteresis": "ﾋｽﾃﾘｼｽ",
  "camera.hysteresis_fields": "ﾋｽﾃﾘｼｽ ｶﾝﾜ",
  "camera.burn_rate": "ﾔｷﾂｷ ｿｸﾄﾞ",
  "camera.burn_recovery": "ﾔｷﾂｷ ｶｲﾌｸ",
  "camera.blemishes": "ｼﾐ/ｷｽﾞ",
  "camera.amp_drive": "ﾌﾟﾘｱﾝﾌﾟ ﾋｾﾝｹｲ",
  "camera.amp_asymmetry": "ｾｲﾌ ﾋﾀｲｼｮｳ",
  "camera.noise_pct": "ﾌﾟﾘｱﾝﾌﾟ ｻﾞﾂｵﾝ",
  "camera.mismatch": "RGB ﾊﾞﾗﾂｷ",
  "camera.aging": "ﾚｯｶ",
  "camera.aperture": "ﾘﾝｶｸﾎｾｲ",
  "camera.knee": "ﾆｰ ｶｲｼ",
  "camera.knee_slope": "ﾆｰ ｶﾀﾑｷ",
  "camera.white_clip": "ﾎﾜｲﾄｸﾘｯﾌﾟ",
  "camera.camera_gamma": "ｶﾞﾝﾏﾎｾｲ",

  "osd.enabled": "OSD ﾋｮｳｼﾞ",
  "osd.mode": "ｺﾞｳｾｲｻｷ",
  "osd.font_index": "TTC ﾊﾞﾝｺﾞｳ",
  "osd.font_dots": "ﾄﾞｯﾄｽｳ",
  "osd.scale": "ﾓｼﾞ ｻｲｽﾞ",
  "osd.opacity": "ﾌﾄｳﾒｲﾄﾞ",

  "tape.enabled": "ﾃｰﾌﾟ",
  "tape.format": "ﾌｫｰﾏｯﾄ",
  "tape.luma_model": "FM ﾓﾃﾞﾙ",
  "tape.fm_sync_mhz": "ｼﾝｸﾁｯﾌﾟ",
  "tape.fm_white_mhz": "ﾎﾜｲﾄﾋﾟｰｸ",
  "tape.fm_channel_mhz": "ﾃﾞﾝｿｳﾛ ﾀｲｲｷ",
  "tape.fm_noise": "RF ﾉｲｽﾞ",
  "tape.writing_speed_mps": "ｶｷｺﾐ ｿｸﾄﾞ",
  "tape.head_spacing_um": "ﾍｯﾄﾞ ﾖｺﾞﾚ",
  "tape.color_under_khz": "ﾃｲｲｷﾍﾝｶﾝ",
  "tape.color_under_level": "ﾃｲｲｷ ﾚﾍﾞﾙ",
  "output.connection": "ｾﾂｿﾞｸ",

  "dv.bitrate_scale": "ﾌｺﾞｳﾘｮｳ",
  "dv.dct_248": "2-4-8 DCT",
  "dv.noise_rate": "ﾊﾞｰｽﾄﾉｲｽﾞ",
  "dv.noise_length": "ﾉｲｽﾞ ﾉ ﾅｶﾞｻ",
  "dv.noise_density": "ﾊﾞｲﾄ ｱﾔﾏﾘﾘﾂ",
  "dv.ecc": "ECC",
  "dv.ecc_miss": "ECC ﾐﾉｶﾞｼ",
  "dv.concealment": "ｴﾗｰ ｼｭｳｾｲ",
  "dv.error_rate": "ﾌﾞﾛｯｸ ｴﾗｰ",
  "dv.burst_rate": "ﾄﾞﾛｯﾌﾟｱｳﾄ",
  "dv.burst_length": "ﾄﾞﾛｯﾌﾟ ﾅｶﾞｻ",
  "dv.head_clog_rate": "ﾍｯﾄﾞ ﾒﾂﾞﾏﾘ",
  "dv.head_clog_frames": "ﾒﾂﾞﾏﾘ ﾅｶﾞｻ",

  "tape.luma_bw_mhz": "ｷﾄﾞ ﾀｲｲｷ",
  "tape.luma_noise_ire": "FM ﾉｲｽﾞ",
  "tape.emphasis_db": "ｴﾝﾌｧｼｽ",
  "tape.emphasis_mhz": "ｴﾝﾌｧｼｽ ｼｭｳﾊｽｳ",
  "tape.white_clip_ire": "ﾎﾜｲﾄｸﾘｯﾌﾟ",
  "tape.bearding_threshold_ire": "ﾍﾞｱﾃﾞｨﾝｸﾞ ｼｷｲ",
  "tape.bearding_strength": "ﾍﾞｱﾃﾞｨﾝｸﾞ",
  "tape.sharpness_db": "ﾋﾟｰｷﾝｸﾞ",

  "tape.chroma_bw_khz": "ｸﾛﾏ ﾀｲｲｷ",
  "tape.chroma_noise_ire": "ｸﾛﾏ ﾉｲｽﾞ",
  "tape.chroma_phase_noise_deg": "ｲｿｳ ﾉｲｽﾞ",
  "tape.chroma_delay_samples": "Y/C ﾁｴﾝ",

  "tape.magnetic_model": "ｼﾞｾｲﾀｲ ﾓﾃﾞﾙ",
  "tape.saturation_ire": "ｲﾛ ﾎｳﾜ",
  "tape.rf_saturation": "RF ﾎｳﾜ",
  "tape.magnetic_media": "ﾃｰﾌﾟ ﾉ ｼｭﾙｲ",
  "tape.rf_drive_hc": "FM ｷﾛｸ ｼﾞｶｲ",
  "audio.bias_level": "ﾊﾞｲｱｽ",

  "tape.dropout_rate": "ﾊｯｾｲﾘﾂ",
  "tape.dropout_length": "ﾍｲｷﾝﾁｮｳ",
  "tape.dropout_depth_um": "ﾃｰﾌﾟ ﾉ ｳｷ",
  "tape.doc_threshold_db": "DOC ｹﾝｼｭﾂ",
  "tape.dropout_compensation": "DOC",

  "tape.crease_rate": "ｼﾜ",
  "tape.crease_fields": "ｼﾜ ﾉ ﾅｶﾞｻ",
  "tape.head_clog_rate": "ﾍｯﾄﾞ ﾒﾂﾞﾏﾘ",

  "transport.jitter_samples": "ｼﾞｯﾀ",
  "transport.jitter_correlation": "ｼﾞｯﾀ ｿｳｶﾝ",
  "transport.wow_samples": "ﾜｳﾌﾗｯﾀ",
  "transport.wow_hz": "ﾜｳ ｼｭｳﾊｽｳ",
  "transport.head_switch_lines": "ﾍｯﾄﾞｷﾘｶｴ ﾗｲﾝ",
  "transport.head_switch_offset": "ﾍｯﾄﾞｷﾘｶｴ ｽﾞﾚ",
  "transport.head_switch_noise": "ﾍｯﾄﾞｷﾘｶｴ ﾉｲｽﾞ",
  "transport.skew_samples": "ｽｷｭｰ",
  "transport.tracking_error": "ﾄﾗｯｷﾝｸﾞ",
  "transport.tracking_speed": "ﾉｲｽﾞﾊﾞｰ ｿｸﾄﾞ",
  "transport.vertical_jitter": "ｽｲﾁｮｸ ﾕﾚ",

  "tbc.enabled": "TBC",
  "tbc.window_samples": "ﾎｾｲ ﾊﾝｲ",
  "tbc.gate_samples": "ｹﾞｰﾄ ﾊﾊﾞ",
  "tbc.reacquire_lines": "ｻｲﾎｿｸ",
  "tbc.error_rate": "ｺﾞｹﾝｼｭﾂ",
  "tbc.residual_samples": "ｻﾞﾝﾘｭｳ ｼﾞｯﾀ",
  "tbc.bits": "A/D ﾋﾞｯﾄ",
  "tbc.regenerate_sync": "ﾄﾞｳｷ ｻｲｾｲｾｲ",

  "rf.enabled": "RF ｾﾂｿﾞｸ",
  "rf.placement": "ｹｲﾛ",
  "rf.snow_ire": "ｽﾉｰ",
  "rf.ghost_gain": "ｺﾞｰｽﾄ",
  "rf.ghost_delay_us": "ｺﾞｰｽﾄ ﾁｴﾝ",
  "rf.interference_ire": "ﾋﾞｰﾄ ﾎﾞｳｶﾞｲ",
  "rf.interference_mhz": "ﾋﾞｰﾄ ｼｭｳﾊｽｳ",
  "rf.sound_beat": "ｵﾝｾｲ ﾋﾞｰﾄ",
  "rf.agc_enabled": "AGC",
  "rf.agc_mode": "AGC ﾎｳｼｷ",
  "rf.signal_db": "ｼﾞｭｼﾝ ﾚﾍﾞﾙ",
  "rf.agc_time_ms": "AGC ｼﾞﾃｲｽｳ",
  "rf.agc_range_db": "AGC ﾊﾝｲ",
  "rf.agc_offset_db": "AGC ｷｼﾞｭﾝ",
  "rf.overload_db": "ｶﾆｭｳﾘｮｸ",
  "rf.flutter_db": "ﾌﾗｯﾀｰ",
  "rf.flutter_hz": "ﾌﾗｯﾀｰ ｼｭｳﾊｽｳ",

  "encoder.enabled": "ｺﾝﾎﾟｼﾞｯﾄ",
  "encoder.luma_bw_mhz": "ｷﾄﾞ ﾀｲｲｷ",
  "encoder.i_bw_mhz": "I ﾀｲｲｷ",
  "encoder.q_bw_mhz": "Q ﾀｲｲｷ",
  "encoder.uv_bw_mhz": "U/V ﾀｲｲｷ",
  "encoder.chroma_gain": "ｲﾛ ｹﾞｲﾝ",
  "encoder.burst_level_pct": "ﾊﾞｰｽﾄ ｼﾝﾌﾟｸ",
  "encoder.setup": "ｾｯﾄｱｯﾌﾟ",
  "encoder.white_point": "ｷｼﾞｭﾝ ｼﾛ",

  "vbi.enabled": "VBI",
  "vbi.closed_caption": "ｼﾞﾏｸ ｿｳｼｭﾂ",
  "vbi.caption_text": "ｼﾞﾏｸ ﾌﾞﾝ",
  "vbi.caption_mode": "ｼﾞﾏｸ ﾓｰﾄﾞ",
  "vbi.caption_channel": "ｼﾞﾏｸ CH",
  "vbi.vits": "VITS",
  "vbi.vitc": "VITC",
  "vbi.teletext": "ﾓｼﾞ ﾀｼﾞｭｳ",
  "vbi.teletext_system": "ﾓｼﾞ ﾀｼﾞｭｳ ﾎｳｼｷ",
  "vbi.teletext_lines": "ﾓｼﾞ ﾀｼﾞｭｳ ﾗｲﾝ",
  "vbi.wss": "WSS/CGMS",
  "signal.vertical_sync.jitter": "V SYNC ｼﾞｯﾀ",
  "signal.vertical_sync.attenuation_pct": "V SYNC ﾚﾍﾞﾙ",
  "signal.vertical_sync.noise_ire": "V SYNC ﾉｲｽﾞ",
  "signal.horizontal_sync.jitter": "H SYNC ｼﾞｯﾀ",
  "signal.horizontal_sync.attenuation_pct": "H SYNC ﾚﾍﾞﾙ",
  "signal.horizontal_sync.noise_ire": "H SYNC ﾉｲｽﾞ",
  "signal.color_burst.jitter": "BURST ｲｿｳ ｼﾞｯﾀ",
  "signal.color_burst.attenuation_pct": "BURST ﾚﾍﾞﾙ",
  "signal.color_burst.noise_ire": "BURST ﾉｲｽﾞ",
  "signal.chroma_subcarrier.jitter": "CHROMA ｲｿｳ ｼﾞｯﾀ",
  "signal.chroma_subcarrier.attenuation_pct": "CHROMA ﾚﾍﾞﾙ",
  "signal.chroma_subcarrier.noise_ire": "CHROMA ﾉｲｽﾞ",

  "decoder.yc_separation": "Y/C ﾌﾞﾝﾘ",
  "decoder.motion_adaptive": "3D ｳｺﾞｷﾃｷｵｳ",
  "decoder.motion_threshold_ire": "3D ｳｺﾞｷ ｼｷｲ",
  "decoder.frame_buffer_bits": "3D ﾊﾞｯﾌｧ ﾋﾞｯﾄ",
  "decoder.chroma_bw_mhz": "ｼｷｻ ﾀｲｲｷ",
  "decoder.setup": "ｼﾞｭｿﾞｳｷ ｾｯﾄｱｯﾌﾟ",
  "decoder.white_point": "ｲﾛｵﾝﾄﾞ",
  "decoder.chroma_demod": "ｲﾛ ﾌｸﾁｮｳ",
  "decoder.narrow_bw_mhz": "ｷｮｳﾀｲｲｷ",
  "decoder.xz_x_axis_deg": "X ｼﾞｸ",
  "decoder.xz_x_gain": "X ｹﾞｲﾝ",
  "decoder.xz_z_axis_deg": "Z ｼﾞｸ",
  "decoder.xz_z_gain": "Z ｹﾞｲﾝ",
  "decoder.xz_cathode_k": "ｶｿｰﾄﾞ ｹﾂｺﾞｳ",
  "decoder.acc": "ACC",
  "decoder.color_killer": "ｶﾗｰｷﾗｰ",
  "decoder.h_pll": "ｽｲﾍｲ AFC",
  "decoder.burst_pll": "ｲﾛ PLL",
  "decoder.vcxo_offset_hz": "ｲﾛ VCXO ｽﾞﾚ",
  "decoder.pal_delay_line": "PAL ﾁｴﾝｾﾝ",
  "decoder.contrast": "ｺﾝﾄﾗｽﾄ",
  "decoder.brightness": "ｱｶﾙｻ",
  "decoder.saturation": "ｲﾛﾉ ｺｻ",
  "decoder.hue_deg": "ｼｷｿｳ",
  "decoder.v_hold_pct": "V HOLD",
  "decoder.v_sync_window": "V ﾋｷｺﾐ",
  "decoder.v_integrator_us": "V ｾｷﾌﾞﾝ",
  "decoder.vertical_position": "V POSITION",
  "decoder.caption_display": "ｼﾞﾏｸ ﾃﾞｺｰﾀﾞ",
  "decoder.caption_channel": "ﾃﾞｺｰﾀﾞ CH",
  "output.deinterlace": "ｲﾝﾀｰﾚｰｽ",
  "output.deinterlace_params.weave_temporal_weight_pct": "WEAVE ﾌｨｰﾙﾄﾞ",
  "output.deinterlace_params.bob_blend_pct": "BOB ﾗｲﾝ ﾎｶﾝ",
  "output.deinterlace_params.blend_strength_pct": "BLEND ﾍｲｷﾝ",
  "output.deinterlace_params.motion_adaptive_threshold": "M-A ｳｺﾞｷ ｼｷｲ",
  "output.deinterlace_params.motion_adaptive_blend_pct": "M-A ﾎｶﾝﾘｮｳ",
  "output.deinterlace_params.yadif_motion_threshold": "YADIF ｳｺﾞｷ",
  "output.deinterlace_params.yadif_search_radius": "YADIF EDGE",
  "output.deinterlace_params.yadif_temporal_weight_pct": "YADIF ｼﾞｶﾝ",
  "output.deinterlace_params.qtgmc_block_size": "QTGMC BLOCK",
  "output.deinterlace_params.qtgmc_search_radius": "QTGMC SEARCH",
  "output.deinterlace_params.qtgmc_temporal_weight_pct": "QTGMC TIME",
  "output.deinterlace_params.qtgmc_sharpness_pct": "QTGMC SHARP",

  "capture.enabled": "ｷｬﾌﾟﾁｬ",
  "capture.levels": "ﾚﾍﾞﾙ",
  "capture.chroma_subsampling": "ｼｷｻ ﾏﾋﾞｷ",
  "capture.adc_bits": "ADC ﾋﾞｯﾄ",
  "capture.codec": "DCT ｱｯｼｭｸ",
  "capture.bitrate_kbps": "ﾋﾞｯﾄﾚｰﾄ",
  "capture.gop": "GOP",
  "capture.motion_search": "ｳｺﾞｷ ﾀﾝｻｸ",
  "capture.rate_control": "ﾚｰﾄｾｲｷﾞｮ",
  "capture.frame_drop_rate": "ｺﾏｵﾁ",

  "audio.enabled": "ｵﾝｾｲ ｶｺｳ",
  "audio.mode": "ｵﾝｾｲ ﾄﾗｯｸ",
  "audio.low_cut_hz": "ﾃｲｲｷ ｶｯﾄ",
  "audio.high_cut_hz": "ｺｳｲｷ ｶｯﾄ",
  "audio.tape_speed": "ﾃｰﾌﾟ ｿｸﾄﾞ",
  "audio.head_gap_um": "ﾍｯﾄﾞ ｷﾞｬｯﾌﾟ",
  "audio.head_spacing_um": "ｵﾝｾｲ ﾍｯﾄﾞ ﾖｺﾞﾚ",
  "audio.drive_db": "ﾛｸｵﾝ ﾚﾍﾞﾙ",
  "audio.hiss_db": "ﾋｽ",
  "audio.wow_pct": "ﾜｳ",
  "audio.flutter_pct": "ﾌﾗｯﾀ",
  "audio.dropout_rate": "ﾄﾞﾛｯﾌﾟｱｳﾄ",
  "audio.hifi_buzz": "Hi-Fi ﾊﾞｽﾞ",
  "audio.hum_db": "ﾃﾞﾝｹﾞﾝ ﾊﾑ",
  "audio.hum_hz": "ﾃﾞﾝｹﾞﾝ ｼｭｳﾊｽｳ",
  "audio.agc": "AGC",
  "audio.sample_rate_hz": "ｻﾝﾌﾟﾘﾝｸﾞ",
  "audio.bits": "ADC ﾋﾞｯﾄ",
  "audio.mono": "ﾓﾉﾗﾙ",
  "audio.codec_kbps": "AAC ﾋﾞｯﾄﾚｰﾄ",
};

/** 選択肢のラベル (path → value → ラベル)。ないものは元のラベルを変換して使う */
export const optionLabels: Record<string, Record<string, string>> = {
  standard: { ntsc: "NTSC", pal: "PAL" },
  "image.fit": { stretch: "ﾋｷﾉﾊﾞｼ", original: "ｵﾘｼﾞﾅﾙ", cover: "ｵｵｳ", contain: "ｵｻﾒﾙ", pixel: "ﾄﾞｯﾄ" },
  "osd.mode": { direct: "ﾀﾞｲﾚｸﾄ", capture: "ｷｬﾌﾟﾁｬ", composite: "ﾃﾞｯｷ ｼｭﾂﾘｮｸ", tape: "ﾃｰﾌﾟ ﾔｷｺﾐ" },
  "tape.format": { beta: "BETA", custom: "ｶｽﾀﾑ", dv: "DV", digital8: "Digital8" },
  "tape.luma_model": { fm: "FM ｹｲｻﾝ", simple: "ｶﾝｲ" },
  "output.connection": { composite: "ｺﾝﾎﾟｼﾞｯﾄ", s_video: "S ｴｲｿﾞｳ" },
  "tape.magnetic_model": { tanh: "tanh ﾎｳﾜ", jiles_atherton: "J-A ﾋｽﾃﾘｼｽ", linear: "ｾﾝｹｲ" },
  "tape.magnetic_media": { cobalt: "ｺﾊﾞﾙﾄ", high_cobalt: "ｺｳ Hc ｺﾊﾞﾙﾄ", metal_particle: "ﾒﾀﾙ", ferric_audio: "ｻﾝｶﾃﾂ" },
  "rf.placement": { after_tape: "ﾃｰﾌﾟ ﾉ ｱﾄ", before_tape: "ﾃｰﾌﾟ ﾉ ﾏｴ", both: "ﾘｮｳﾎｳ" },
  "decoder.yc_separation": { notch: "ﾉｯﾁ", comb: "ｸｼｶﾞﾀ", comb3d: "3ｼﾞｹﾞﾝ" },
  "output.deinterlace": {
    weave: "WEAVE",
    bob: "BOB",
    blend: "BLENDING",
    motion_adaptive_blend: "M-A BLEND",
    yadif: "YADIFMOD2",
    qtgmc: "QTGMC KANI",
  },
  "capture.levels": { correct: "ﾀﾀﾞｼｲ", washed_out: "ｸﾛｳｷ", crushed: "ｸﾛﾂﾌﾞﾚ" },
  "audio.mode": { linear: "ﾘﾆｱ", hifi: "Hi-Fi" },
};

/** プリセット名 (params.ts の presets の name → ラベル) */
export const presetLabels: Record<string, string> = {
  "VHS 標準 (SP)": "VHS ﾋｮｳｼﾞｭﾝ SP",
  "90 年代の家庭用ビデオカメラ": "90ﾈﾝﾀﾞｲ ﾋﾞﾃﾞｵｶﾒﾗ",
  "放送用プランビコン 3 管カメラ": "ﾌﾟﾗﾝﾋﾞｺﾝ 3ｶﾝ",
  "家庭用ビジコンカメラ": "ﾋﾞｼﾞｺﾝ ｶﾒﾗ",
  "サチコンカメラ": "ｻﾁｺﾝ ｶﾒﾗ",
  "劣化した撮像管カメラ": "ﾚｯｶ ｻﾂｿﾞｳｶﾝ",
  "VHS 3倍 (EP)": "VHS 3ﾊﾞｲ EP",
  "劣化したテープ": "ﾚｯｶ ﾃｰﾌﾟ",
  "TBC 付き S-VHS デッキ": "S-VHS + TBC",
  "TBC が暴れる劣化テープ": "TBC ｴﾗｰ ﾃｰﾌﾟ",
  "Hi-Fi デッキ": "Hi-Fi ﾃﾞｯｷ",
  "RF 接続のブラウン管": "RF ﾌﾞﾗｳﾝｶﾝ",
  "電波の弱い放送を録画": "ｼﾞｬｸﾃﾞﾝｶｲ ﾛｸｶﾞ",
  "DVD レコーダーでダビング": "DVD ﾀﾞﾋﾞﾝｸﾞ",
  "2000年代の取り込み動画 (低ビットレート)": "2000ﾈﾝﾀﾞｲ ﾄﾞｳｶﾞ",
  "転載を重ねたネット動画": "ﾃﾝｻｲ ﾄﾞｳｶﾞ",
  "放送受信 (テープなし)": "ﾎｳｿｳ ｼﾞｭｼﾝ",
  "MiniDV カムコーダ (FireWire 取り込み)": "MiniDV FireWire",
  "MiniDV をアナログ出力 (S 映像)": "MiniDV S ｴｲｿﾞｳ",
  "劣化した MiniDV テープ": "ﾚｯｶ MiniDV",
  "壊れた DV ファイル (誤り訂正なし)": "ｺﾜﾚﾀ DV",
  "撮像管カメラだけ (信号処理なし)": "ｶﾒﾗ ﾉﾐ",
  "PAL VHS (シンプル PAL 受像機)": "PAL VHS",
};
