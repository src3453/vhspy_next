// オシロスコープが今どのラインを見ているか (デコード画面のオーバーレイ用に共有する)。

export type ScopeStandard = "ntsc" | "pal";

export const scopeView = $state({
  /** オシロスコープが開いていて、画面への表示が有効なとき true */
  active: false,
  standard: "ntsc" as ScopeStandard,
  /** 表示を始めるライン番号 (1 始まり) */
  line: 21,
  /** 表示するライン数 */
  count: 1,
});

/** vhs-core の timing.rs と同じ値 */
const RASTER: Record<ScopeStandard, { total: number; fieldLines: number; activeFirst: [number, number]; field1Last: number }> = {
  ntsc: { total: 525, fieldLines: 240, activeFirst: [22, 285], field1Last: 263 },
  pal: { total: 625, fieldLines: 288, activeFirst: [23, 336], field1Last: 312 },
};

/** 画像の高さ (フレームのライン数) */
export const imageLines = (std: ScopeStandard) => RASTER[std].fieldLines * 2;

/** ライン番号 → 画像の行 (映像のないラインは null)。raster.rs の image_row と同じ */
export function imageRow(std: ScopeStandard, n: number): number | null {
  const r = RASTER[std];
  for (let f = 0; f < 2; f++) {
    const a = n - r.activeFirst[f];
    if (a >= 0 && a < r.fieldLines) return 2 * a + f;
  }
  return null;
}

/** ライン番号がどちらのフィールドか (0 = 第 1, 1 = 第 2) */
export const fieldOf = (std: ScopeStandard, n: number) => (n <= RASTER[std].field1Last ? 0 : 1);

/** 映像のないラインが、そのフィールドの映像より前 (画面の上) か後ろ (下) か */
export const offscreenAbove = (std: ScopeStandard, n: number) => n < RASTER[std].activeFirst[fieldOf(std, n)];

/** 選んでいるライン番号の列 */
export function selectedLines(std: ScopeStandard, line: number, count: number): number[] {
  const t = RASTER[std].total;
  return Array.from({ length: Math.min(count, t) }, (_, k) => ((line - 1 + k) % t) + 1);
}
