<script lang="ts">
  // コンポジット信号のオシロスコープ (ライン選択付きの波形モニタ)。
  // 受像機に入る直前の信号 (帰線区間・同期・VBI のデータ込み) を、ライン番号で選んで描く。
  // 開いている間だけエンジンに信号を写してもらう。
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy } from "svelte";

  type Standard = "ntsc" | "pal";
  let { standard = "ntsc" as Standard } = $props();

  // ---- 表示設定 (localStorage に保存) ----
  type Span = "1" | "2" | "3" | "9" | "22" | "v";
  type Filter = "flat" | "lpass" | "chroma";
  type Saved = { open: boolean; line: Record<Standard, number>; span: Span; filter: Filter; persist: boolean };
  const STORE_KEY = "vhs-oscilloscope";
  function load(): Saved {
    const d: Saved = { open: false, line: { ntsc: 21, pal: 23 }, span: "1", filter: "flat", persist: true };
    try {
      return { ...d, ...JSON.parse(localStorage.getItem(STORE_KEY) ?? "{}") };
    } catch {
      return d;
    }
  }
  const saved = load();
  let open = $state(saved.open);
  let lines = $state(saved.line);
  let span = $state<Span>(saved.span);
  let filter = $state<Filter>(saved.filter);
  let persist = $state(saved.persist);
  $effect(() => {
    const s = JSON.stringify({ open, line: lines, span, filter, persist });
    try {
      localStorage.setItem(STORE_KEY, s);
    } catch {}
  });

  const TOTAL: Record<Standard, number> = { ntsc: 525, pal: 625 };
  const total = $derived(TOTAL[standard]);
  const line = $derived(Math.min(Math.max(1, lines[standard] ?? 1), TOTAL[standard]));
  function setLine(v: number) {
    const t = TOTAL[standard];
    const n = ((((Math.round(v) - 1) % t) + t) % t) + 1;
    lines = { ...lines, [standard]: n };
  }

  /** 表示するライン数 */
  const count = $derived(span === "v" ? (standard === "ntsc" ? 263 : 313) : Number(span));

  // ---- 受け取った信号 (第 1 / 第 2 フィールドのバッファを別々に持つ) ----
  type Buf = {
    id: number;
    pal: boolean;
    parity: number;
    frame: number;
    raster: number;
    ls: number;
    first: number;
    total: number;
    separate: boolean;
    fs: number;
    syncEnd: number;
    burstStart: number;
    burstEnd: number;
    activeStart: number;
    activeWidth: number;
    y: Float32Array;
    c: Float32Array | null;
  };
  let bufs: (Buf | null)[] = [null, null];
  let latest: Buf | null = $state(null);
  let lastId = 0;

  function parse(ab: ArrayBuffer): Buf | null {
    if (ab.byteLength < 68) return null;
    const dv = new DataView(ab);
    const u = (o: number) => dv.getUint32(o, true);
    const raster = u(24),
      ls = u(28);
    const n = raster * ls;
    const separate = u(40) === 1;
    if (ab.byteLength < 68 + n * 4 * (separate ? 2 : 1)) return null;
    return {
      id: Number(dv.getBigUint64(0, true)),
      pal: u(8) === 1,
      parity: u(12),
      frame: Number(dv.getBigUint64(16, true)),
      raster,
      ls,
      first: u(32),
      total: u(36),
      separate,
      fs: dv.getFloat32(44, true),
      syncEnd: u(48),
      burstStart: u(52),
      burstEnd: u(56),
      activeStart: u(60),
      activeWidth: u(64),
      y: new Float32Array(ab, 68, n),
      c: separate ? new Float32Array(ab, 68 + n * 4, n) : null,
    };
  }

  /** ライン番号 n (1 始まり) の信号。新しいフィールドのバッファから探す */
  function lineData(n: number): { b: Buf; at: number } | null {
    const order = [bufs[0], bufs[1]].filter((b): b is Buf => !!b).sort((a, b) => b.id - a.id);
    for (const b of order) {
      const i = (((n - b.first) % b.total) + b.total) % b.total;
      if (i < b.raster) return { b, at: i * b.ls };
    }
    return null;
  }

  // ---- 取得ループ ----
  let timer: ReturnType<typeof setTimeout> | null = null;
  let busy = false;
  async function poll() {
    timer = null;
    if (!open) return;
    if (!busy) {
      busy = true;
      try {
        const ab = await invoke<ArrayBuffer>("get_composite", { after: lastId });
        const b = ab && ab.byteLength ? parse(ab) : null;
        if (b && (b.pal ? "pal" : "ntsc") === standard) {
          lastId = b.id;
          bufs[b.parity & 1] = b;
          latest = b;
          draw();
        } else if (b) {
          lastId = b.id;
        }
      } catch {
        // エンジンが止まっている間などは次で取り直す
      } finally {
        busy = false;
      }
    }
    if (open) timer = setTimeout(poll, 40);
  }

  $effect(() => {
    const active = open;
    invoke("set_scope_active", { active }).catch(() => {});
    if (active) {
      bufs = [null, null];
      lastId = 0;
      poll();
    } else if (timer) {
      clearTimeout(timer);
      timer = null;
    }
  });
  onDestroy(() => {
    if (timer) clearTimeout(timer);
    invoke("set_scope_active", { active: false }).catch(() => {});
  });
  // 方式が変わったら古い信号を捨てる
  $effect(() => {
    void standard;
    bufs = [null, null];
    latest = null;
  });

  // ---- 縦軸 [IRE] ----
  const IRE_HI = 130;
  const IRE_LO = -50;
  const GUTTER = 44;
  const FOOT = 18;

  let dataCanvas: HTMLCanvasElement | undefined = $state();
  let gridCanvas: HTMLCanvasElement | undefined = $state();

  function fit(c: HTMLCanvasElement, clear = true) {
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(c.clientWidth * dpr));
    const h = Math.max(1, Math.round(c.clientHeight * dpr));
    let resized = false;
    if (c.width !== w || c.height !== h) {
      c.width = w;
      c.height = h;
      resized = true;
    }
    const ctx = c.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    if (clear) ctx.clearRect(0, 0, c.clientWidth, c.clientHeight);
    return { ctx, w: c.clientWidth, h: c.clientHeight, resized };
  }

  const yOf = (ire: number, h: number) => ((IRE_HI - ire) / (IRE_HI - IRE_LO)) * (h - FOOT);

  /** 表示の 1 サンプル (フィルタ込み)。LPASS は副搬送波 1 周期 (4 サンプル) の平均で色を落とす */
  function sampleAt(b: Buf, at: number, k: number): number {
    const v = (i: number) => {
      const j = Math.min(Math.max(i, 0), b.ls - 1);
      return b.y[at + j] + (b.c ? b.c[at + j] : 0);
    };
    if (filter === "flat") return v(k);
    const lp = 0.125 * v(k - 2) + 0.25 * (v(k - 1) + v(k) + v(k + 1)) + 0.125 * v(k + 2);
    return filter === "lpass" ? lp : v(k) - lp;
  }

  function draw() {
    if (!dataCanvas || !open) return;
    const { ctx, w, h, resized } = fit(dataCanvas, false);
    const x0 = GUTTER;
    const pw = w - x0;
    if (persist && !resized) {
      // 蛍光面の残光
      ctx.fillStyle = "rgba(0,0,0,0.35)";
      ctx.fillRect(0, 0, w, h);
    } else {
      ctx.clearRect(0, 0, w, h);
    }
    const ref = latest;
    if (!ref) return;
    const ls = ref.ls;
    const n = count;
    const totalSamples = n * ls;
    const off = filter === "chroma" ? 50 : 0;
    ctx.strokeStyle = "rgba(130,255,160,0.9)";
    ctx.lineWidth = 1.1;
    ctx.beginPath();
    if (totalSamples <= pw * 2) {
      // 折れ線
      let started = false;
      for (let l = 0; l < n; l++) {
        const src = lineData(line + l);
        if (!src) {
          started = false;
          continue;
        }
        for (let k = 0; k < ls; k++) {
          const x = x0 + ((l * ls + k) / totalSamples) * pw;
          const y = yOf(sampleAt(src.b, src.at, k) + off, h);
          if (!started) {
            ctx.moveTo(x, y);
            started = true;
          } else ctx.lineTo(x, y);
        }
      }
    } else {
      // 1 画素の列ごとの最小・最大 (包絡線)
      const cols = Math.max(1, Math.floor(pw));
      let prevLo = NaN,
        prevHi = NaN;
      let cache: { l: number; src: ReturnType<typeof lineData> } = { l: -1, src: null };
      for (let x = 0; x < cols; x++) {
        const s0 = Math.floor((x / cols) * totalSamples);
        const s1 = Math.max(s0 + 1, Math.floor(((x + 1) / cols) * totalSamples));
        let lo = Infinity,
          hi = -Infinity;
        for (let s = s0; s < s1; s++) {
          const l = Math.floor(s / ls);
          if (cache.l !== l) cache = { l, src: lineData(line + l) };
          if (!cache.src) continue;
          const v = sampleAt(cache.src.b, cache.src.at, s - l * ls) + off;
          if (v < lo) lo = v;
          if (v > hi) hi = v;
        }
        if (lo > hi) {
          prevLo = prevHi = NaN;
          continue;
        }
        // 隣の列とつなげる
        if (!Number.isNaN(prevLo)) {
          lo = Math.min(lo, prevHi);
          hi = Math.max(hi, prevLo);
        }
        const xx = x0 + x + 0.5;
        ctx.moveTo(xx, yOf(hi, h));
        ctx.lineTo(xx, yOf(lo, h) + 0.6);
        prevLo = lo;
        prevHi = hi;
      }
    }
    ctx.stroke();
  }

  // ---- 目盛り ----
  function lineLabel(std: Standard, n: number): string {
    if (std === "ntsc") {
      if ((n >= 1 && n <= 3) || (n >= 7 && n <= 9) || (n >= 264 && n <= 265) || (n >= 270 && n <= 272)) return "等化パルス";
      if ((n >= 4 && n <= 6) || (n >= 266 && n <= 269)) return "垂直同期";
      if (n === 21 || n === 284) return "字幕 (EIA-608)";
      if (n === 17 || n === 280) return "VITS 合成試験信号";
      if (n === 18 || n === 281) return "VITS マルチバースト";
      if ([14, 16, 277, 279].includes(n)) return "VITC";
      if (n === 20 || n === 283) return "CGMS-A";
      if ((n >= 10 && n <= 20) || (n >= 273 && n <= 283)) return "VBI";
      if (n === 263) return "半ライン | 等化";
      return n <= 263 ? "第 1 フィールド" : "第 2 フィールド";
    }
    if ((n >= 1 && n <= 3) || (n >= 313 && n <= 315)) return "垂直同期";
    if ((n >= 4 && n <= 5) || (n >= 311 && n <= 312) || (n >= 316 && n <= 318) || n >= 624) return "等化パルス";
    if (n === 23) return "WSS | 半ライン";
    if (n === 17 || n === 330) return "ITS 合成試験信号";
    if (n === 18 || n === 331) return "ITS マルチバースト";
    if ([19, 21, 332, 334].includes(n)) return "VITC";
    if ((n >= 6 && n <= 22) || (n >= 319 && n <= 335)) return "VBI (テレテキスト)";
    if (n === 623) return "半ライン | 等化";
    return n <= 312 ? "第 1 フィールド" : "第 2 フィールド";
  }

  function drawGrid() {
    if (!gridCanvas) return;
    const { ctx, w, h } = fit(gridCanvas);
    const x0 = GUTTER;
    const pw = w - x0;
    const ph = h - FOOT;
    ctx.font = "10px 'Segoe UI', sans-serif";
    ctx.lineWidth = 1;
    ctx.textBaseline = "middle";
    // 縦軸
    for (let ire = IRE_LO; ire <= IRE_HI; ire += 10) {
      const y = Math.round(yOf(ire, h)) + 0.5;
      const major = ire % 20 === 0;
      ctx.strokeStyle = ire === 0 ? "rgba(255,200,80,0.45)" : major ? "rgba(255,200,80,0.25)" : "rgba(255,200,80,0.11)";
      ctx.beginPath();
      ctx.moveTo(x0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
      if (major) {
        ctx.textAlign = "right";
        ctx.fillStyle = "rgba(255,210,120,0.75)";
        ctx.fillText(String(ire), x0 - 6, y);
      }
    }
    const pal = standard === "pal";
    const levels: [number, string, string][] = pal
      ? [
          [100, "白 100 (700 mV)", "rgba(255,255,255,0.5)"],
          [-300 / 7, "同期 -300 mV", "rgba(255,140,140,0.6)"],
        ]
      : [
          [100, "白 100 IRE", "rgba(255,255,255,0.5)"],
          [7.5, "セットアップ 7.5", "rgba(120,200,255,0.7)"],
          [-40, "同期 -40 IRE", "rgba(255,140,140,0.6)"],
        ];
    for (const [ire, label, color] of levels) {
      const y = Math.round(yOf(ire, h)) + 0.5;
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
    }
    ctx.fillStyle = "rgba(255,210,120,0.75)";
    ctx.textAlign = "left";
    ctx.fillText("IRE", 4, 9);

    // 横軸
    const ls = pal ? 1135 : 910;
    const fs = pal ? 4 * 4_433_618.75 : (4 * 315e6) / 88;
    const n = count;
    const lineUs = (ls / fs) * 1e6;
    ctx.textBaseline = "alphabetic";
    if (n <= 3) {
      // 1 µs ごとの小目盛りと 10 µs ごとの数字、ライン内の区間 (同期・バースト・映像)
      const seg = pal
        ? { sync: 83, bs: 100, be: 140, as: 186, aw: 922 }
        : { sync: 67, bs: 76, be: 112, as: 135, aw: 754 };
      for (let l = 0; l < n; l++) {
        const xl = (k: number) => x0 + ((l * ls + k) / (n * ls)) * pw;
        const band = (a: number, b: number, color: string) => {
          ctx.fillStyle = color;
          ctx.fillRect(xl(a), ph, xl(b) - xl(a), 4);
        };
        band(0, seg.sync, "rgba(255,120,120,0.7)");
        band(seg.bs, seg.be, "rgba(255,210,80,0.8)");
        band(seg.as, seg.as + seg.aw, "rgba(120,200,255,0.5)");
        for (let t = 0; t < lineUs; t += 1) {
          const x = Math.round(xl((t * 1e-6 * fs))) + 0.5;
          const major = t % 10 === 0;
          ctx.strokeStyle = major ? "rgba(255,200,80,0.25)" : "rgba(255,200,80,0.08)";
          ctx.beginPath();
          ctx.moveTo(x, major ? 0 : ph - 6);
          ctx.lineTo(x, ph);
          ctx.stroke();
          if (major) {
            ctx.fillStyle = "rgba(255,210,120,0.7)";
            ctx.textAlign = "center";
            ctx.fillText(`${t}`, x, h - 3);
          }
        }
        if (l > 0) {
          ctx.strokeStyle = "rgba(255,255,255,0.3)";
          const x = Math.round(xl(0)) + 0.5;
          ctx.beginPath();
          ctx.moveTo(x, 0);
          ctx.lineTo(x, ph);
          ctx.stroke();
        }
      }
      ctx.textAlign = "left";
      ctx.fillStyle = "rgba(255,255,255,0.45)";
      ctx.fillText("µs", 4, h - 3);
    } else {
      // ライン番号の目盛り
      const step = n <= 22 ? 1 : n <= 60 ? 5 : 20;
      for (let l = 0; l < n; l++) {
        const num = ((line - 1 + l) % total) + 1;
        const x = Math.round(x0 + (l / n) * pw) + 0.5;
        const major = num % step === 0 || n <= 22;
        ctx.strokeStyle = major ? "rgba(255,200,80,0.2)" : "rgba(255,200,80,0.06)";
        ctx.beginPath();
        ctx.moveTo(x, major ? 0 : ph - 5);
        ctx.lineTo(x, ph);
        ctx.stroke();
        if (num % step === 0 || (n <= 22 && (n <= 12 || l % 2 === 0))) {
          ctx.fillStyle = "rgba(255,210,120,0.7)";
          ctx.textAlign = n <= 22 ? "left" : "center";
          ctx.fillText(String(num), n <= 22 ? x + 2 : x, h - 3);
        }
      }
      ctx.textAlign = "left";
      ctx.fillStyle = "rgba(255,255,255,0.45)";
      ctx.fillText("ライン", 2, h - 3);
    }
  }

  // 開いている間は、方式・範囲・表示サイズが変わるたびに目盛りを描き直す
  $effect(() => {
    void standard;
    void span;
    void line;
    if (!open || !gridCanvas || !dataCanvas) return;
    drawGrid();
    draw();
    const ro = new ResizeObserver(() => {
      drawGrid();
      draw();
    });
    ro.observe(gridCanvas);
    return () => ro.disconnect();
  });
  $effect(() => {
    void filter;
    void persist;
    draw();
  });

  // ---- カーソル読み取り ----
  let readout = $state("");
  function onMove(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    const r = el.getBoundingClientRect();
    const x = e.clientX - r.left - GUTTER;
    const y = e.clientY - r.top;
    const pw = r.width - GUTTER;
    if (x < 0 || x > pw || !latest) {
      readout = "";
      return;
    }
    const ls = latest.ls;
    const s = (x / pw) * count * ls;
    const l = Math.floor(s / ls);
    const k = s - l * ls;
    const num = ((line - 1 + l) % total) + 1;
    const ire = IRE_HI - (y / (r.height - FOOT)) * (IRE_HI - IRE_LO);
    const src = lineData(num);
    const v = src ? sampleAt(src.b, src.at, Math.floor(k)) : NaN;
    readout =
      `ライン ${num} · ${((k / latest.fs) * 1e6).toFixed(2)} µs · カーソル ${ire.toFixed(1)} IRE` +
      (Number.isFinite(v) ? ` · 信号 ${v.toFixed(1)} IRE` : "");
  }
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    setLine(line + (e.deltaY > 0 ? 1 : -1) * (e.shiftKey ? 10 : 1));
  }
</script>

<section class="osc" class:open>
  <header>
    <button class="toggle" onclick={() => (open = !open)} aria-expanded={open}>
      <span class="chev">{open ? "▾" : "▸"}</span> オシロスコープ (コンポジット)
    </button>
    {#if open}
      <label title="表示を始めるライン番号 (ホイールでも変えられる。Shift で 10 ライン)">
        ライン
        <button class="step" onclick={() => setLine(line - 1)} aria-label="前のライン">◀</button>
        <input
          type="number"
          min="1"
          max={total}
          value={line}
          onchange={(e) => setLine(Number((e.target as HTMLInputElement).value))}
        />
        <button class="step" onclick={() => setLine(line + 1)} aria-label="次のライン">▶</button>
      </label>
      <span class="what">{lineLabel(standard, line)}</span>
      <select bind:value={span} title="表示するライン数">
        <option value="1">1H</option>
        <option value="2">2H</option>
        <option value="3">3H</option>
        <option value="9">9H</option>
        <option value="22">22H (帰線区間)</option>
        <option value="v">1V (フィールド)</option>
      </select>
      <select bind:value={filter} title="波形モニタのフィルタ">
        <option value="flat">FLAT</option>
        <option value="lpass">LPASS (輝度)</option>
        <option value="chroma">CHROMA (+50 IRE)</option>
      </select>
      <label class="check" title="蛍光面の残光">
        <input type="checkbox" bind:checked={persist} /> 残光
      </label>
      <span class="presets">
        <button onclick={() => { setLine(standard === "ntsc" ? 1 : 309); span = "22"; }} title="垂直帰線区間">V 帰線</button>
        {#if standard === "ntsc"}
          <button onclick={() => { setLine(21); span = "1"; }} title="ライン 21 の字幕">字幕</button>
        {/if}
        <button onclick={() => { setLine(17); span = "2"; }} title="挿入試験信号">VITS</button>
      </span>
      {#if latest?.separate}<span class="note">S 映像: Y + C を合成して表示</span>{/if}
    {/if}
  </header>
  {#if open}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="screen" style:--gutter="{GUTTER}px" onpointermove={onMove} onpointerleave={() => (readout = "")} onwheel={onWheel}>
      <canvas class="grid" bind:this={gridCanvas}></canvas>
      <canvas class="data" bind:this={dataCanvas}></canvas>
      {#if !latest}<div class="wait">信号を待っています… (コンポジット信号を通しているときだけ表示)</div>{/if}
      {#if readout}<div class="readout">{readout}</div>{/if}
    </div>
  {/if}
</section>

<style>
  .osc {
    border-top: 1px solid var(--line);
    background: #0c0d10;
  }
  header {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
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
  label {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  input[type="number"] {
    width: 58px;
  }
  .step {
    padding: 0 5px;
  }
  .what {
    color: #ffd27a;
    min-width: 9em;
  }
  .presets {
    display: flex;
    gap: 4px;
  }
  .note {
    color: #9cf;
  }
  .screen {
    position: relative;
    height: 240px;
    margin: 0 8px 8px;
    background: #000;
    border: 1px solid var(--line);
    border-radius: 3px;
    overflow: hidden;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
  }
  .grid {
    pointer-events: none;
  }
  .data {
    pointer-events: none;
    mix-blend-mode: screen;
  }
  .wait,
  .readout {
    position: absolute;
    left: calc(var(--gutter) + 8px);
    color: rgba(255, 255, 255, 0.6);
    font-size: 11px;
    pointer-events: none;
  }
  .wait {
    top: 8px;
  }
  .readout {
    top: 6px;
    background: rgba(0, 0, 0, 0.6);
    padding: 1px 6px;
    border-radius: 3px;
  }
</style>
