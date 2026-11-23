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
  output: { deinterlace: "weave" | "bob" };
  seed: number;
};

export type Control =
  | { kind: "range"; path: string; label: string; min: number; max: number; step: number; unit?: string }
  | { kind: "bool"; path: string; label: string }
  | { kind: "select"; path: string; label: string; options: { value: string; label: string }[] };

export type Group = { title: string; note?: string; controls: Control[] };

const r = (path: string, label: string, min: number, max: number, step: number, unit?: string): Control => ({
  kind: "range", path, label, min, max, step, unit,
});
const b = (path: string, label: string): Control => ({ kind: "bool", path, label });

export const groups: Group[] = [
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
];

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
    },
  },
  {
    name: "劣化したテープ",
    diff: {
      tape: { luma_bw_mhz: 2.0, luma_noise_ire: 6, chroma_bw_khz: 300, chroma_noise_ire: 4, chroma_phase_noise_deg: 10, saturation_ire: 25, dropout_rate: 0.02, dropout_length: 80 },
      transport: { jitter_samples: 1.5, wow_samples: 2, head_switch_lines: 8, head_switch_offset: 20, tracking_error: 0.35 },
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
