<script lang="ts">
  // 映像波形 (IRE 目盛り) とベクトルスコープ。プレビューと同じ RGBA フレームから描く。
  // 格納中は計算しない。

  type Standard = "ntsc" | "pal";
  let { standard = "ntsc" as Standard } = $props();

  const STORE_KEY = "vhs-scopes";
  type Saved = { open: boolean; mode: "luma" | "parade"; gain: number };
  function load(): Saved {
    const d: Saved = { open: false, mode: "luma", gain: 1 };
    try {
      return { ...d, ...JSON.parse(localStorage.getItem(STORE_KEY) ?? "{}") };
    } catch {
      return d;
    }
  }
  const saved = load();
  let open = $state(saved.open);
  let mode = $state<Saved["mode"]>(saved.mode);
  let gain = $state(saved.gain);
  $effect(() => {
    const s = JSON.stringify({ open, mode, gain });
    try {
      localStorage.setItem(STORE_KEY, s);
    } catch {}
  });

  // ---- 信号レベル (vhs-core の timing.rs と同じ値) ----
  // デコーダは Y = (IRE - 黒) / (白 - 黒) で RGB を作るので、その逆で IRE に戻す
  const LEVELS: Record<Standard, { black: number; white: number; sync: number }> = {
    ntsc: { black: 7.5, white: 100, sync: -40 },
    pal: { black: 0, white: 100, sync: -300 / 7 },
  };
  // 波形の縦軸の範囲 [IRE]
  const IRE_HI = 110;
  const IRE_LO = -10;

  // 内部解像度
  const WW = 512;
  const WH = 240;
  const VS = 256;

  let waveCanvas: HTMLCanvasElement | undefined = $state();
  let waveGrid: HTMLCanvasElement | undefined = $state();
  let vecCanvas: HTMLCanvasElement | undefined = $state();
  let vecGrid: HTMLCanvasElement | undefined = $state();

  const waveAcc = new Uint32Array(WW * WH * 3);
  const vecAcc = new Uint32Array(VS * VS);
  const vecLevel = new Float32Array(VS * VS);
  let waveImg: ImageData | null = null;
  let vecImg: ImageData | null = null;
  let lastTime = 0;

  /** IRE → 波形の縦位置 (0 = 上端, 1 = 下端) */
  const ireToFrac = (ire: number) => (IRE_HI - ire) / (IRE_HI - IRE_LO);

  // ベクトルスコープ: Cb/Cr (BT.601) を ±VEC_RANGE で枠いっぱいにする
  const VEC_RANGE = 0.6;
  const KB = 0.564;
  const KR = 0.713;
  function cbcr(r: number, g: number, b: number): [number, number] {
    const y = 0.299 * r + 0.587 * g + 0.114 * b;
    return [KB * (b - y), KR * (r - y)];
  }

  /** プレビューのループから毎フレーム呼ばれる。格納中・間引き中は何もしない */
  export function push(data: Uint8ClampedArray, w: number, h: number) {
    if (!open || !waveCanvas || !vecCanvas) return;
    const now = performance.now();
    if (now - lastTime < 33) return;
    lastTime = now;
    compute(data, w, h);
    render();
  }

  // 8 bit 値 → 波形の行 (IRE に換算して縦軸へ)
  const rowLut = new Int32Array(256);
  let lutStd: Standard | null = null;
  function buildLut() {
    if (lutStd === standard) return;
    lutStd = standard;
    const lv = LEVELS[standard];
    for (let v = 0; v < 256; v++) {
      const ire = lv.black + (v / 255) * (lv.white - lv.black);
      rowLut[v] = Math.min(WH - 1, Math.max(0, Math.round(ireToFrac(ire) * (WH - 1))));
    }
  }

  let samples = 1;
  function compute(d: Uint8ClampedArray, w: number, h: number) {
    buildLut();
    waveAcc.fill(0);
    vecAcc.fill(0);
    const parade = mode === "parade";
    const third = Math.floor(WW / 3);
    const colBin = new Int32Array(w);
    const colBinP = new Int32Array(w);
    for (let x = 0; x < w; x++) {
      colBin[x] = Math.floor((x * WW) / w);
      colBinP[x] = Math.floor((x * third) / w);
    }
    const half = VS / 2;
    const vk = (half - 1) / VEC_RANGE / 255;
    // 1 ライン置きに間引く (インタレースの片フィールド相当)
    let n = 0;
    for (let y = 0; y < h; y += 2) {
      let o = y * w * 4;
      for (let x = 0; x < w; x++, o += 4) {
        const r = d[o],
          g = d[o + 1],
          b = d[o + 2];
        if (parade) {
          const xb = colBinP[x];
          waveAcc[(rowLut[r] * WW + xb) * 3]++;
          waveAcc[(rowLut[g] * WW + xb + third) * 3 + 1]++;
          waveAcc[(rowLut[b] * WW + xb + 2 * third) * 3 + 2]++;
        } else {
          const yy = (0.299 * r + 0.587 * g + 0.114 * b + 0.5) | 0;
          waveAcc[(rowLut[yy] * WW + colBin[x]) * 3]++;
        }
        const yl = 0.299 * r + 0.587 * g + 0.114 * b;
        const px = (half + KB * (b - yl) * vk) | 0;
        const py = (half - KR * (r - yl) * vk) | 0;
        if (px >= 0 && px < VS && py >= 0 && py < VS) vecAcc[py * VS + px]++;
        n++;
      }
    }
    samples = Math.max(1, n);
  }

  function render() {
    const wctx = waveCanvas!.getContext("2d")!;
    const vctx = vecCanvas!.getContext("2d")!;
    waveImg ??= wctx.createImageData(WW, WH);
    vecImg ??= vctx.createImageData(VS, VS);

    // 列ごとのサンプル数で正規化 (列の全サンプルが 1 点に集まると 1)
    const parade = mode === "parade";
    const perCol = samples / (parade ? Math.floor(WW / 3) : WW);
    const kw = (gain * 12) / perCol;
    const wd = waveImg.data;
    for (let i = 0, o = 0; i < WW * WH; i++, o += 4) {
      const a = waveAcc[i * 3],
        b = waveAcc[i * 3 + 1],
        c = waveAcc[i * 3 + 2];
      if (parade) {
        wd[o] = 255 * (1 - Math.exp(-a * kw));
        wd[o + 1] = 255 * (1 - Math.exp(-b * kw));
        wd[o + 2] = 255 * (1 - Math.exp(-c * kw));
      } else {
        const v = 1 - Math.exp(-a * kw);
        wd[o] = 150 * v;
        wd[o + 1] = 255 * v;
        wd[o + 2] = 170 * v;
      }
      wd[o + 3] = 255;
    }
    wctx.putImageData(waveImg, 0, 0);

    const kv = (gain * VS * VS * 0.15) / samples;
    for (let i = 0; i < VS * VS; i++) vecLevel[i] = 1 - Math.exp(-vecAcc[i] * kv);
    // 蛍光面のにじみ: 上下左右の明るさを少し広げて、ノイズのない絵でも点が見えるようにする
    const vd = vecImg.data;
    for (let i = 0, o = 0; i < VS * VS; i++, o += 4) {
      const x = i % VS;
      let nb = 0;
      if (x > 0) nb = Math.max(nb, vecLevel[i - 1]);
      if (x < VS - 1) nb = Math.max(nb, vecLevel[i + 1]);
      if (i >= VS) nb = Math.max(nb, vecLevel[i - VS]);
      if (i < VS * (VS - 1)) nb = Math.max(nb, vecLevel[i + VS]);
      const v = Math.max(vecLevel[i], nb * 0.6);
      vd[o] = 140 * v;
      vd[o + 1] = 255 * v;
      vd[o + 2] = 160 * v;
      vd[o + 3] = 255;
    }
    vctx.putImageData(vecImg, 0, 0);
  }

  // ---- 目盛り (表示サイズに合わせて高解像度で描く) ----
  function fitCanvas(c: HTMLCanvasElement) {
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(c.clientWidth * dpr));
    const h = Math.max(1, Math.round(c.clientHeight * dpr));
    if (c.width !== w || c.height !== h) {
      c.width = w;
      c.height = h;
    }
    const ctx = c.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, c.clientWidth, c.clientHeight);
    return { ctx, w: c.clientWidth, h: c.clientHeight };
  }

  const GUTTER = 44; // 波形の左側 (IRE ラベル) の幅 [CSS px]

  function drawWaveGrid() {
    if (!waveGrid) return;
    const { ctx, w, h } = fitCanvas(waveGrid);
    const lv = LEVELS[standard];
    const x0 = GUTTER;
    const yOf = (ire: number) => Math.round(ireToFrac(ire) * (h - 1)) + 0.5;
    ctx.font = "10px 'Segoe UI', sans-serif";
    ctx.textBaseline = "middle";
    ctx.textAlign = "right";
    ctx.lineWidth = 1;

    for (let ire = IRE_LO; ire <= IRE_HI; ire += 10) {
      const y = yOf(ire);
      const major = ire % 20 === 0;
      ctx.strokeStyle = major ? "rgba(255,200,80,0.28)" : "rgba(255,200,80,0.13)";
      ctx.beginPath();
      ctx.moveTo(x0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
      if (ire > IRE_LO && ire < IRE_HI && (major || ire === -10)) {
        ctx.fillStyle = "rgba(255,210,120,0.75)";
        ctx.fillText(String(ire), x0 - 6, y);
      }
    }
    // 小目盛り (2 IRE) を左端に
    ctx.strokeStyle = "rgba(255,200,80,0.35)";
    for (let ire = IRE_LO; ire <= IRE_HI; ire += 2) {
      const y = yOf(ire);
      const len = ire % 10 === 0 ? 8 : 4;
      ctx.beginPath();
      ctx.moveTo(x0, y);
      ctx.lineTo(x0 + len, y);
      ctx.stroke();
    }
    // 白・黒 (セットアップ) レベル
    const mark = (ire: number, label: string, color: string) => {
      const y = yOf(ire);
      ctx.strokeStyle = color;
      ctx.setLineDash([5, 4]);
      ctx.beginPath();
      ctx.moveTo(x0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.fillStyle = color;
      ctx.textAlign = "right";
      ctx.fillText(label, w - 4, y - 7);
    };
    mark(lv.white, `白 ${lv.white} IRE`, "rgba(255,255,255,0.55)");
    if (lv.black !== 0) mark(lv.black, `セットアップ ${lv.black} IRE`, "rgba(120,200,255,0.75)");
    else mark(0, "黒 0 IRE", "rgba(120,200,255,0.75)");

    // 単位と方式
    ctx.textAlign = "left";
    ctx.fillStyle = "rgba(255,210,120,0.75)";
    ctx.fillText("IRE", 4, 9);
    ctx.fillStyle = "rgba(255,255,255,0.45)";
    const mv = standard === "ntsc" ? "1 IRE ≈ 7.14 mV" : "1 IRE = 7 mV";
    ctx.fillText(`${standard.toUpperCase()} · 同期 ${lv.sync.toFixed(1)} IRE · ${mv}`, x0 + 12, h - 9);

    // パレードの区切り
    if (mode === "parade") {
      ctx.strokeStyle = "rgba(255,255,255,0.25)";
      for (const f of [1 / 3, 2 / 3]) {
        const x = Math.round(x0 + (w - x0) * f) + 0.5;
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x, h);
        ctx.stroke();
      }
      ctx.textAlign = "center";
      const names: [string, string][] = [
        ["R", "#ff7a7a"],
        ["G", "#7aff8a"],
        ["B", "#7aa8ff"],
      ];
      names.forEach(([t, c], i) => {
        ctx.fillStyle = c;
        ctx.fillText(t, x0 + ((w - x0) * (i + 0.5)) / 3, 9);
      });
    }
  }

  function drawVecGrid() {
    if (!vecGrid) return;
    const { ctx, w, h } = fitCanvas(vecGrid);
    const cx = w / 2,
      cy = h / 2;
    const R = (Math.min(w, h) / 2) * ((VS / 2 - 1) / (VS / 2));
    const pos = (cb: number, cr: number): [number, number] => [cx + (cb / VEC_RANGE) * R, cy - (cr / VEC_RANGE) * R];
    ctx.lineWidth = 1;
    ctx.font = "10px 'Segoe UI', sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";

    // 外周 (100% 彩度の赤までの距離) と十字
    const rFull = Math.hypot(...cbcr(1, 0, 0)) / VEC_RANGE * R;
    ctx.strokeStyle = "rgba(255,200,80,0.25)";
    ctx.beginPath();
    ctx.arc(cx, cy, rFull, 0, Math.PI * 2);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(cx - rFull, cy);
    ctx.lineTo(cx + rFull, cy);
    ctx.moveTo(cx, cy - rFull);
    ctx.lineTo(cx, cy + rFull);
    ctx.stroke();
    // 中心付近の目盛り (10% 刻み)
    for (let i = 1; i < 10; i++) {
      const t = (rFull * i) / 10;
      ctx.beginPath();
      ctx.moveTo(cx + t, cy - 3);
      ctx.lineTo(cx + t, cy + 3);
      ctx.moveTo(cx - t, cy - 3);
      ctx.lineTo(cx - t, cy + 3);
      ctx.moveTo(cx - 3, cy + t);
      ctx.lineTo(cx + 3, cy + t);
      ctx.moveTo(cx - 3, cy - t);
      ctx.lineTo(cx + 3, cy - t);
      ctx.stroke();
    }

    // 肌色ライン (I 軸, 123°)
    const a = (123 * Math.PI) / 180;
    ctx.strokeStyle = "rgba(255,170,120,0.35)";
    ctx.setLineDash([4, 4]);
    ctx.beginPath();
    ctx.moveTo(cx, cy);
    ctx.lineTo(cx + Math.cos(a) * rFull, cy - Math.sin(a) * rFull);
    ctx.stroke();
    ctx.setLineDash([]);

    // 75% / 100% カラーバーのターゲット
    const bars: [string, number, number, number][] = [
      ["R", 1, 0, 0],
      ["Mg", 1, 0, 1],
      ["B", 0, 0, 1],
      ["Cy", 0, 1, 1],
      ["G", 0, 1, 0],
      ["Yl", 1, 1, 0],
    ];
    for (const [name, r, g, b] of bars) {
      const [cb75, cr75] = cbcr(0.75 * r, 0.75 * g, 0.75 * b);
      const [x, y] = pos(cb75, cr75);
      const s = Math.max(6, R * 0.05);
      ctx.strokeStyle = "rgba(255,200,80,0.7)";
      ctx.strokeRect(x - s / 2, y - s / 2, s, s);
      const [cb1, cr1] = cbcr(r, g, b);
      const [x1, y1] = pos(cb1, cr1);
      ctx.strokeStyle = "rgba(255,200,80,0.4)";
      ctx.beginPath();
      ctx.arc(x1, y1, 2.5, 0, Math.PI * 2);
      ctx.stroke();
      // ラベルは 100% 点の外側
      const d = Math.hypot(x1 - cx, y1 - cy) || 1;
      ctx.fillStyle = "rgba(255,210,120,0.85)";
      ctx.fillText(name, x1 + ((x1 - cx) / d) * 11, y1 + ((y1 - cy) / d) * 11);
    }
    ctx.textAlign = "left";
    ctx.fillStyle = "rgba(255,255,255,0.45)";
    ctx.fillText("□75% ○100%", 4, h - 9);
  }

  function drawGrids() {
    drawWaveGrid();
    drawVecGrid();
  }

  // 開いている間は、方式・モード・表示サイズが変わるたびに目盛りを描き直す
  $effect(() => {
    void standard;
    void mode;
    if (!open || !waveGrid || !vecGrid) return;
    const ro = new ResizeObserver(drawGrids);
    ro.observe(waveGrid);
    ro.observe(vecGrid);
    return () => ro.disconnect();
  });
</script>

<section class="scopes" class:open>
  <header>
    <button class="toggle" onclick={() => (open = !open)} aria-expanded={open}>
      <span class="chev">{open ? "▾" : "▸"}</span> スコープ
    </button>
    {#if open}
      <select bind:value={mode} title="波形の表示">
        <option value="luma">輝度 (Y)</option>
        <option value="parade">RGB パレード</option>
      </select>
      <label class="gain" title="輝度 (ダブルクリックで既定値)">
        明るさ
        <input
          type="range"
          min="-2"
          max="2"
          step="0.05"
          value={Math.log2(gain)}
          oninput={(e) => (gain = 2 ** Number((e.target as HTMLInputElement).value))}
          ondblclick={() => (gain = 1)}
        />
      </label>
    {/if}
  </header>
  {#if open}
    <div class="body">
      <div class="wave" style:--gutter="{GUTTER}px">
        <canvas class="data" bind:this={waveCanvas} width={WW} height={WH}></canvas>
        <canvas class="grid" bind:this={waveGrid}></canvas>
      </div>
      <div class="vec">
        <canvas class="data" bind:this={vecCanvas} width={VS} height={VS}></canvas>
        <canvas class="grid" bind:this={vecGrid}></canvas>
      </div>
    </div>
  {/if}
</section>

<style>
  .scopes {
    border-top: 1px solid var(--line);
    background: #0c0d10;
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 8px;
    color: var(--muted);
  }
  .toggle {
    background: none;
    border: none;
    padding: 2px 4px;
    color: var(--text);
    font-weight: 600;
    cursor: pointer;
  }
  .chev {
    display: inline-block;
    width: 1em;
  }
  .gain {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .gain input {
    width: 110px;
  }
  .body {
    display: flex;
    gap: 8px;
    height: 240px;
    padding: 0 8px 8px;
  }
  .wave,
  .vec {
    position: relative;
    background: #000;
    border: 1px solid var(--line);
    border-radius: 3px;
    overflow: hidden;
  }
  .wave {
    flex: 1;
    min-width: 0;
  }
  .vec {
    aspect-ratio: 1;
    height: 100%;
  }
  canvas {
    position: absolute;
    top: 0;
    height: 100%;
    display: block;
  }
  .wave .data {
    left: var(--gutter);
    width: calc(100% - var(--gutter));
  }
  .vec .data,
  .grid {
    left: 0;
    width: 100%;
  }
  .data {
    image-rendering: auto;
  }
  .grid {
    pointer-events: none;
  }
</style>
