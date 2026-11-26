<script lang="ts">
  // 生信号ビュー (MMSSTV の Sync 画面のようなもの)。
  // 受像機に入る直前のコンポジット信号をデコードせずに画像にし、帰線区間・同期・VBI も含めて全部見せる。
  // 縦は信号の垂直同期で揃え、横は同期を取らない (ライン長をずらすと水平同期の帯が斜めになる)。
  // 画素は信号のレベル [IRE] で、疑似カラーとレベル (表示する範囲) はここで決める。
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, untrack } from "svelte";
  import { imageRow } from "#lib/scopeView.svelte.ts";

  type Standard = "ntsc" | "pal";
  let { standard = "ntsc" as Standard, open = false } = $props();

  // ---- 表示設定 (localStorage に保存) ----
  /** gray = 白黒、bipolar = 帰線レベル (0 IRE) を黒にして + を暖色・- を青、heat = 黒→赤→黄→白、rainbow = 等高線のような虹色 */
  type Palette = "gray" | "bipolar" | "heat" | "rainbow";
  /** 揃える垂直同期: f1 = 第 1 フィールド、f2 = 第 2 フィールド、either = どちらでも (新しい方) */
  type Anchor = "f1" | "f2" | "either";
  type Saved = {
    frame: boolean;
    anchor: Anchor;
    sync: boolean;
    /** ライン長のずれ [サンプル] (方式ごと) */
    slant: Record<Standard, number>;
    /** 横の位置 [サンプル] (方式ごと) */
    phase: Record<Standard, number>;
    preLines: number;
    invert: boolean;
    fps: number;
    palette: Palette;
    /** 表示する範囲 [IRE] (これより下・上は端の色) */
    levelLo: number;
    levelHi: number;
  };
  const STORE_KEY = "vhs-syncview";
  function load(): Saved {
    const d: Saved = {
      frame: true,
      anchor: "f1",
      sync: true,
      slant: { ntsc: 0, pal: 0 },
      phase: { ntsc: 0, pal: 0 },
      preLines: 3,
      invert: false,
      fps: 30,
      palette: "bipolar",
      levelLo: -45,
      levelHi: 105,
    };
    try {
      return { ...d, ...JSON.parse(localStorage.getItem(STORE_KEY) ?? "{}") };
    } catch {
      return d;
    }
  }
  const saved = load();
  let frame = $state(saved.frame);
  let anchor = $state<Anchor>(saved.anchor);
  let sync = $state(saved.sync);
  let slants = $state(saved.slant);
  let phases = $state(saved.phase);
  let preLines = $state(saved.preLines);
  let invert = $state(saved.invert);
  let fps = $state(saved.fps);
  let palette = $state<Palette>(saved.palette);
  let levelLo = $state(saved.levelLo);
  let levelHi = $state(saved.levelHi);
  $effect(() => {
    const s = JSON.stringify({ frame, anchor, sync, slant: slants, phase: phases, preLines, invert, fps, palette, levelLo, levelHi });
    try {
      localStorage.setItem(STORE_KEY, s);
    } catch {}
  });

  const LS_DEFAULT: Record<Standard, number> = { ntsc: 910, pal: 1135 };
  const FS: Record<Standard, number> = { ntsc: (4 * 315e6) / 88, pal: 4 * 4_433_618.75 };
  /** ラインのサンプル数 (受け取った画像から。NTSC の 228 周期ラインなら 912) */
  let ls = $state(untrack(() => LS_DEFAULT[standard]));
  // 方式を変えたら、画像が届くまで規格の値にしておく
  $effect(() => {
    ls = LS_DEFAULT[standard];
  });
  const slant = $derived(slants[standard] ?? 0);
  const phase = $derived(phases[standard] ?? 0);
  const width = $derived(ls + slant);
  /** 今のライン長に当たる水平周波数 */
  const fh = $derived(FS[standard] / width);
  function setSlant(v: number) {
    slants = { ...slants, [standard]: Math.round(Math.max(-ls / 2, Math.min(ls / 2, v)) * 1000) / 1000 };
  }
  function setPhase(v: number) {
    const w = width;
    phases = { ...phases, [standard]: Math.round((((v % w) + w) % w) * 100) / 100 };
  }

  // ---- 受け取った画像 ----
  type Img = {
    id: number;
    width: number;
    height: number;
    /** 0 = フリーラン、1 = 垂直同期で揃えた、2 = 見失ったので前の位置を保っている */
    lock: 0 | 1 | 2;
    pal: boolean;
    total: number;
    firstLine: number;
    linesPerRow: number;
    fs: number;
    /** 画素 0 と 255 に当たるレベル [IRE] */
    rangeLo: number;
    rangeHi: number;
    gray: Uint8Array;
  };
  const HEADER = 56;
  function parse(ab: ArrayBuffer): Img | null {
    if (ab.byteLength < HEADER) return null;
    const dv = new DataView(ab);
    const w = dv.getUint32(8, true);
    const h = dv.getUint32(12, true);
    if (ab.byteLength < HEADER + w * h) return null;
    return {
      id: Number(dv.getBigUint64(0, true)),
      width: w,
      height: h,
      lock: Math.min(2, dv.getUint32(16, true)) as 0 | 1 | 2,
      pal: dv.getUint32(20, true) === 1,
      total: dv.getUint32(24, true),
      firstLine: dv.getFloat64(28, true),
      linesPerRow: dv.getFloat64(36, true),
      fs: dv.getFloat32(44, true),
      rangeLo: dv.getFloat32(48, true),
      rangeHi: dv.getFloat32(52, true),
      gray: new Uint8Array(ab, HEADER, w * h),
    };
  }

  let canvas: HTMLCanvasElement | undefined = $state();
  let overlay: HTMLCanvasElement | undefined = $state();
  let last: Img | null = $state.raw(null);
  let lastId = 0;
  /** 設定が変わったので、新しいフィールドがなくても描き直す */
  let dirty = true;
  /** 垂直同期を見失ったときに保つ位置 (最後に揃えた画像の 1 行目のライン番号) */
  let holdLine: number | null = null;

  // ---- 疑似カラー ----
  type RGB = [number, number, number];
  const clamp01 = (x: number) => (x < 0 ? 0 : x > 1 ? 1 : x);
  /** 区分線形のカラーマップ */
  function ramp(stops: [number, RGB][], t: number): RGB {
    t = clamp01(t);
    for (let i = 1; i < stops.length; i++) {
      const [t1, c1] = stops[i];
      if (t <= t1) {
        const [t0, c0] = stops[i - 1];
        const u = t1 > t0 ? (t - t0) / (t1 - t0) : 0;
        return [c0[0] + (c1[0] - c0[0]) * u, c0[1] + (c1[1] - c0[1]) * u, c0[2] + (c1[2] - c0[2]) * u];
      }
    }
    return stops[stops.length - 1][1];
  }
  const HEAT: [number, RGB][] = [
    [0, [0, 0, 0]],
    [0.35, [0.75, 0.05, 0.02]],
    [0.65, [1, 0.6, 0.05]],
    [0.85, [1, 0.95, 0.4]],
    [1, [1, 1, 1]],
  ];
  const RAINBOW: [number, RGB][] = [
    [0, [0.1, 0, 0.35]],
    [0.15, [0.15, 0.2, 1]],
    [0.35, [0, 0.85, 1]],
    [0.5, [0.1, 0.9, 0.2]],
    [0.68, [1, 0.95, 0.1]],
    [0.85, [1, 0.4, 0]],
    [1, [0.9, 0, 0.1]],
  ];
  /** - 側: 黒 → 紺 → 青 → 水色 (同期の先端ほど明るい) */
  const NEG: [number, RGB][] = [
    [0, [0, 0, 0]],
    [0.35, [0.05, 0.12, 0.55]],
    [0.75, [0.1, 0.45, 1]],
    [1, [0.55, 0.95, 1]],
  ];
  /** + 側: 黒 → 焦げ茶 → 橙 → クリーム → 白 */
  const POS: [number, RGB][] = [
    [0, [0, 0, 0]],
    [0.3, [0.4, 0.2, 0.06]],
    [0.65, [0.9, 0.62, 0.3]],
    [1, [1, 0.98, 0.92]],
  ];

  /** レベル [IRE] → 色 (0..1) */
  function colorOf(ire: number): RGB {
    const lo = Math.min(levelLo, levelHi - 1);
    const hi = Math.max(levelHi, lo + 1);
    const t = (ire - lo) / (hi - lo);
    let c: RGB;
    switch (palette) {
      case "gray": {
        const v = clamp01(t);
        c = [v, v, v];
        break;
      }
      case "heat":
        c = ramp(HEAT, t);
        break;
      case "rainbow":
        c = ramp(RAINBOW, t);
        break;
      default: {
        // 帰線レベル (0 IRE) を黒にして、+ と - を別の色で伸ばす。範囲の片側が 0 を越えていなければ全体を片側の色にする
        if (ire < 0) c = lo < 0 ? ramp(NEG, ire / lo) : [0, 0, 0];
        else c = hi > 0 ? ramp(POS, ire / hi) : [0, 0, 0];
      }
    }
    return invert ? [1 - c[0], 1 - c[1], 1 - c[2]] : c;
  }

  /** 画素値 (0..255) → RGBA の表。範囲とパレットが変わったときだけ作る */
  let lut = new Uint8ClampedArray(256 * 4);
  let lutKey = "";
  function ensureLut(img: Img) {
    const key = `${palette}|${levelLo}|${levelHi}|${invert}|${img.rangeLo}|${img.rangeHi}`;
    if (key === lutKey) return;
    lutKey = key;
    for (let i = 0; i < 256; i++) {
      const c = colorOf(img.rangeLo + (i / 255) * (img.rangeHi - img.rangeLo));
      lut[i * 4] = c[0] * 255 + 0.5;
      lut[i * 4 + 1] = c[1] * 255 + 0.5;
      lut[i * 4 + 2] = c[2] * 255 + 0.5;
      lut[i * 4 + 3] = 255;
    }
  }
  const ireOf = (img: Img, code: number) => img.rangeLo + (code / 255) * (img.rangeHi - img.rangeLo);

  function draw(img: Img) {
    if (!canvas) return;
    if (canvas.width !== img.width || canvas.height !== img.height) {
      canvas.width = img.width;
      canvas.height = img.height;
    }
    const ctx = canvas.getContext("2d")!;
    const id = ctx.createImageData(img.width, img.height);
    ensureLut(img);
    const px = new Uint32Array(id.data.buffer);
    const lut32 = new Uint32Array(lut.buffer);
    const g = img.gray;
    for (let i = 0; i < g.length; i++) px[i] = lut32[g[i]];
    ctx.putImageData(id, 0, 0);
    drawOverlay();
    drawLegend();
  }

  // ---- 取得のループ ----
  let timer: ReturnType<typeof setTimeout> | null = null;
  let busy = false;
  async function tick() {
    timer = null;
    if (!open) return;
    const started = performance.now();
    if (!busy) {
      busy = true;
      try {
        const after = dirty ? 0 : lastId;
        dirty = false;
        const ab = await invoke<ArrayBuffer>("get_sync_view", {
          after,
          width,
          phase,
          frame,
          anchor: anchor === "f1" ? 0 : anchor === "f2" ? 1 : 2,
          sync,
          preLines,
          hold: holdLine,
        });
        const img = parse(ab);
        if (img && (img.pal ? "pal" : "ntsc") === standard) {
          lastId = img.id;
          // ライン長はずれ (slant) で持つので、ラインのサンプル数が変わっても設定はそのまま
          if (img.width !== ls) {
            ls = img.width;
            dirty = true;
          }
          if (img.lock === 1) holdLine = img.firstLine;
          last = img;
          draw(img);
        }
      } catch {
        // エンジンが止まっている間などは次で取り直す
      } finally {
        busy = false;
      }
    }
    if (open && timer === null) {
      const period = 1000 / Math.max(1, fps);
      timer = setTimeout(tick, Math.max(0, period - (performance.now() - started)));
    }
  }

  // 開閉だけに反応する
  $effect(() => {
    const active = open;
    untrack(() => {
      invoke("set_scope_active", { active, who: 2 }).catch(() => {});
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
      if (active) {
        lastId = 0;
        dirty = true;
        holdLine = null;
        last = null;
        timer = setTimeout(tick, 0);
      }
    });
  });
  onDestroy(() => {
    if (timer) clearTimeout(timer);
    invoke("set_scope_active", { active: false, who: 2 }).catch(() => {});
  });

  // 設定を変えたら描き直す
  $effect(() => {
    void width;
    void phase;
    void frame;
    void anchor;
    void sync;
    void preLines;
    void standard;
    dirty = true;
    // 保っていた位置は前の設定のものなので捨てる
    holdLine = null;
  });
  // 色とレベルは手元で描き直すだけ
  $effect(() => {
    void invert;
    void palette;
    void levelLo;
    void levelHi;
    untrack(() => last && draw(last));
  });

  // ---- レベル ----
  const LEVEL_PRESETS: { label: string; lo: number; hi: number; title: string }[] = [
    { label: "標準", lo: -45, hi: 105, title: "同期の先端から白まで" },
    { label: "全域", lo: -60, hi: 140, title: "受け取った範囲すべて (同期の下・スーパーホワイトの上も)" },
    { label: "同期", lo: -50, hi: 10, title: "同期とその周り (同期の深さ・等化パルス・セレーションを見る)" },
    { label: "映像", lo: 0, hi: 110, title: "帰線レベルから白まで (映像とバースト・VBI のデータ)" },
  ];
  function setLevels(lo: number, hi: number) {
    levelLo = Math.round(Math.max(-60, Math.min(139, lo)));
    levelHi = Math.round(Math.max(levelLo + 1, Math.min(140, hi)));
  }
  /** 今の画像の 0.2 % 〜 99.8 % に合わせる */
  function autoLevels() {
    const img = last;
    if (!img) return;
    const hist = new Uint32Array(256);
    for (let i = 0; i < img.gray.length; i++) hist[img.gray[i]]++;
    const n = img.gray.length;
    let acc = 0,
      lo = 0,
      hi = 255;
    for (let i = 0; i < 256; i++) {
      acc += hist[i];
      if (acc >= n * 0.002) {
        lo = i;
        break;
      }
    }
    acc = 0;
    for (let i = 255; i >= 0; i--) {
      acc += hist[i];
      if (acc >= n * 0.002) {
        hi = i;
        break;
      }
    }
    setLevels(Math.floor(ireOf(img, lo)), Math.ceil(ireOf(img, hi)));
  }

  // ---- 色の凡例 ----
  let legend: HTMLCanvasElement | undefined = $state();
  function drawLegend() {
    if (!legend || !last) return;
    const W = 256;
    if (legend.width !== W) {
      legend.width = W;
      legend.height = 1;
    }
    const ctx = legend.getContext("2d")!;
    const id = ctx.createImageData(W, 1);
    // 凡例は表示範囲 (levelLo..levelHi) を左から右へ
    for (let x = 0; x < W; x++) {
      const c = colorOf(levelLo + ((x + 0.5) / W) * (levelHi - levelLo));
      id.data[x * 4] = c[0] * 255;
      id.data[x * 4 + 1] = c[1] * 255;
      id.data[x * 4 + 2] = c[2] * 255;
      id.data[x * 4 + 3] = 255;
    }
    ctx.putImageData(id, 0, 0);
  }
  /** 凡例の上で 0 IRE (帰線) の位置 [%] */
  const zeroAt = $derived(((0 - levelLo) / Math.max(1, levelHi - levelLo)) * 100);

  // ---- 目盛り (左にライン番号、右に区間の色) ----
  const GUTTER = 40;
  const BAND = 6;
  /** 行 r の先頭のライン番号 (1 始まり、小数) */
  const rowLine = (img: Img, r: number) => (((img.firstLine - 1 + r * img.linesPerRow) % img.total) + img.total) % img.total + 1;

  function region(std: Standard, n: number): "sync" | "vbi" | "active" {
    if (imageRow(std, n) !== null) return "active";
    const vsyncish =
      std === "ntsc"
        ? n <= 9 || (n >= 263 && n <= 272)
        : n <= 5 || (n >= 310 && n <= 318) || n >= 623;
    return vsyncish ? "sync" : "vbi";
  }
  const REGION_COLOR = { sync: "rgba(255,110,110,0.85)", vbi: "rgba(255,210,80,0.85)", active: "rgba(110,190,255,0.6)" };

  function drawOverlay() {
    if (!overlay) return;
    const dpr = window.devicePixelRatio || 1;
    const cw = overlay.clientWidth,
      ch = overlay.clientHeight;
    const w = Math.max(1, Math.round(cw * dpr)),
      h = Math.max(1, Math.round(ch * dpr));
    if (overlay.width !== w || overlay.height !== h) {
      overlay.width = w;
      overlay.height = h;
    }
    const ctx = overlay.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, cw, ch);
    const img = last;
    if (!img) return;
    const rowH = ch / img.height;
    ctx.font = "10px 'Segoe UI', sans-serif";
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    const step = img.height > 400 ? 25 : 10;
    let prevN = -1;
    for (let r = 0; r < img.height; r++) {
      const n = Math.floor(rowLine(img, r));
      // 区間の色
      ctx.fillStyle = REGION_COLOR[region(standard, n)];
      ctx.fillRect(cw - BAND, r * rowH, BAND, rowH + 0.5);
      // ライン番号
      if (n !== prevN && (n % step === 0 || n === 1)) {
        const y = (r + 0.5) * rowH;
        ctx.fillStyle = n === 1 ? "#ff8a8a" : "rgba(255,210,120,0.8)";
        ctx.fillText(String(n), GUTTER - 6, y);
        ctx.strokeStyle = "rgba(255,210,120,0.35)";
        ctx.beginPath();
        ctx.moveTo(GUTTER - 4, Math.round(y) + 0.5);
        ctx.lineTo(GUTTER, Math.round(y) + 0.5);
        ctx.stroke();
      }
      prevN = n;
    }
  }

  $effect(() => {
    if (!open || !overlay) return;
    const ro = new ResizeObserver(() => drawOverlay());
    ro.observe(overlay);
    return () => ro.disconnect();
  });

  // ---- 自動調整 ----
  /** 行ごとに水平同期 (いちばん低い所) の位置を探す。戻り値は列の位置 (列数を法として連続にしたもの) */
  function syncColumns(img: Img): { r: number; x: number }[] {
    const W = img.width;
    const box = Math.max(4, Math.round(W * 0.06)); // 同期パルス程度の幅
    const out: { r: number; x: number }[] = [];
    let prev: number | null = null;
    for (let r = 0; r < img.height; r++) {
      // 映像のあるラインだけ (垂直同期の幅広パルスは使わない)
      if (region(standard, Math.floor(rowLine(img, r))) !== "active") continue;
      const row = img.gray.subarray(r * W, (r + 1) * W);
      let s = 0;
      for (let k = 0; k < box; k++) s += row[k];
      let best = s,
        bx = 0;
      for (let x = 1; x < W; x++) {
        s += row[(x + box - 1) % W] - row[x - 1];
        if (s < best) {
          best = s;
          bx = x;
        }
      }
      // 低い所が同期レベル付近でなければ使わない
      if (ireOf(img, best / box) > -20) continue;
      let x = bx;
      if (prev !== null) {
        while (x - prev > W / 2) x -= W;
        while (prev - x > W / 2) x += W;
      }
      prev = x;
      out.push({ r, x });
    }
    return out;
  }

  /** 水平同期が左端に来るように横の位置を合わせる */
  function autoPhase() {
    if (!last) return;
    const pts = syncColumns(last);
    if (!pts.length) return;
    const xs = pts.map((p) => ((p.x % last!.width) + last!.width) % last!.width).sort((a, b) => a - b);
    const med = xs[xs.length >> 1];
    setPhase(phase + (med * width) / last.width - 4);
  }

  /** 水平同期の帯の傾きからライン長を合わせる */
  function autoSlant() {
    if (!last) return;
    const pts = syncColumns(last);
    if (pts.length < 10) return;
    const n = pts.length;
    const mr = pts.reduce((a, p) => a + p.r, 0) / n;
    const mx = pts.reduce((a, p) => a + p.x, 0) / n;
    let num = 0,
      den = 0;
    for (const p of pts) {
      num += (p.r - mr) * (p.x - mx);
      den += (p.r - mr) ** 2;
    }
    if (!den) return;
    // 1 行あたり右へ d サンプルずれる = 本当のライン長は今より d 長い
    const d = (num / den) * (width / last.width);
    setSlant(slant + d);
  }

  // ---- カーソル読み取り ----
  let readout = $state("");
  function onMove(e: PointerEvent) {
    const img = last;
    if (!img || !canvas) return;
    const r = canvas.getBoundingClientRect();
    const fx = (e.clientX - r.left) / r.width;
    const fy = (e.clientY - r.top) / r.height;
    if (fx < 0 || fx >= 1 || fy < 0 || fy >= 1) {
      readout = "";
      return;
    }
    const row = Math.floor(fy * img.height);
    const col = Math.floor(fx * img.width);
    const pos = rowLine(img, row) + fx * img.linesPerRow;
    const n = Math.floor(((pos - 1) % img.total + img.total) % img.total) + 1;
    const us = (((pos - Math.floor(pos)) * ls) / img.fs) * 1e6;
    const ire = ireOf(img, img.gray[row * img.width + col]);
    readout = `ライン ${n} · ${us.toFixed(2)} µs · ${ire >= 0 ? "+" : ""}${ire.toFixed(1)} IRE`;
  }
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const k = e.shiftKey ? 1 : e.ctrlKey ? 0.01 : 0.1;
    setSlant(slant + (e.deltaY > 0 ? -k : k));
  }
</script>

{#if open}
  <section class="syncview">
    <header>
      <span class="seg" role="group" aria-label="表示範囲">
        <button class:on={!frame} onclick={() => (frame = false)} title="1 フィールドぶん見せる">1V</button>
        <button class:on={frame} onclick={() => (frame = true)} title="1 フレーム (2 フィールド) ぶん見せる">2V</button>
      </span>
      <span class="seg" role="group" aria-label="揃えるフィールド">
        <button class:on={anchor === "f1"} onclick={() => (anchor = "f1")} title="第 1 フィールドの垂直同期で揃える (毎回同じ絵になる)">F1</button>
        <button class:on={anchor === "f2"} onclick={() => (anchor = "f2")} title="第 2 フィールドの垂直同期で揃える">F2</button>
        <button class:on={anchor === "either"} onclick={() => (anchor = "either")} title="新しい方の垂直同期で揃える (更新ごとに第 1・第 2 フィールドが入れ替わる)">交互</button>
      </span>
      <span class="seg" role="group" aria-label="同期">
        <button class:on={sync} onclick={() => (sync = true)} title="信号の垂直同期で縦を揃える">VSYNC</button>
        <button class:on={!sync} onclick={() => (sync = false)} title="同期を取らない (最新の信号の末尾に合わせる)">フリーラン</button>
      </span>
      <label title="1 行のサンプル数。ずらすと水平同期の帯が斜めになる (ホイールで ±0.1、Shift で ±1、Ctrl で ±0.01)">
        ライン長
        <button class="step" onclick={() => setSlant(slant - 0.1)} aria-label="短く">◀</button>
        <input
          type="number"
          step="0.01"
          value={width.toFixed(2)}
          onchange={(e) => setSlant(Number((e.target as HTMLInputElement).value) - ls)}
        />
        <button class="step" onclick={() => setSlant(slant + 0.1)} aria-label="長く">▶</button>
        <span class="num">{fh.toFixed(2)} Hz</span>
      </label>
      <label class="slider" title="横の位置 (ダブルクリックで 0)">
        位相
        <input
          type="range"
          min="0"
          max={width}
          step="0.5"
          value={phase}
          oninput={(e) => setPhase(Number((e.target as HTMLInputElement).value))}
          ondblclick={() => setPhase(0)}
        />
      </label>
      <label title="垂直同期の前後に見せるライン数">
        余白
        <input type="number" min="0" max="50" step="1" bind:value={preLines} class="short" />
      </label>
      <label title="表示の更新の速さ">
        更新
        <select bind:value={fps}>
          <option value={5}>5 fps</option>
          <option value={10}>10 fps</option>
          <option value={15}>15 fps</option>
          <option value={30}>30 fps</option>
          <option value={60}>60 fps</option>
        </select>
      </label>
      <span class="presets">
        <button onclick={autoPhase} title="水平同期が左端に来るように横の位置を合わせる">Phase</button>
        <button onclick={autoSlant} title="水平同期の帯の傾きからライン長を合わせる">Slant</button>
        <button onclick={() => setSlant(0)} title="ライン長を規格の値に戻す">規定長</button>
      </span>
      {#if last}
        <span class="lock" class:locked={last.lock === 1} class:hold={last.lock === 2}
          title={last.lock === 2 ? "垂直同期を見失ったので、最後に揃えた位置を保っています" : ""}
          >{last.lock === 1 ? "● V ロック" : last.lock === 2 ? "◐ 保持" : "○ フリーラン"}</span
        >
      {/if}
    </header>
    <div class="levels">
      <label title="色の付け方">
        色
        <select bind:value={palette}>
          <option value="bipolar">± (帰線で分ける)</option>
          <option value="gray">白黒</option>
          <option value="heat">ヒート</option>
          <option value="rainbow">レインボー</option>
        </select>
      </label>
      <label title="表示範囲の下端 [IRE] (これより低いところは下端の色)">
        下
        <input type="number" step="1" value={levelLo} onchange={(e) => setLevels(Number((e.target as HTMLInputElement).value), levelHi)} class="short" />
      </label>
      <div class="legend" title="色とレベルの対応 (白い線が 0 IRE = 帰線レベル)">
        <canvas bind:this={legend}></canvas>
        {#if zeroAt > 0 && zeroAt < 100}<span class="zero" style:left="{zeroAt}%"></span>{/if}
      </div>
      <label title="表示範囲の上端 [IRE] (これより高いところは上端の色)">
        上
        <input type="number" step="1" value={levelHi} onchange={(e) => setLevels(levelLo, Number((e.target as HTMLInputElement).value))} class="short" />
      </label>
      <span class="presets">
        {#each LEVEL_PRESETS as p (p.label)}
          <button class:on={levelLo === p.lo && levelHi === p.hi} onclick={() => setLevels(p.lo, p.hi)} title={p.title}>{p.label}</button>
        {/each}
        <button onclick={autoLevels} title="今の画像のレベルの分布に合わせる">自動</button>
        <button class:on={invert} onclick={() => (invert = !invert)} title="色を反転">反転</button>
      </span>
    </div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="screen" style:--gutter="{GUTTER}px" style:--band="{BAND}px" onpointermove={onMove} onpointerleave={() => (readout = "")} onwheel={onWheel}>
      <canvas class="img" bind:this={canvas}></canvas>
      <canvas class="overlay" bind:this={overlay}></canvas>
      {#if !last}
        <div class="wait">信号を待っています… (コンポジット信号を通しているときだけ表示)</div>
      {/if}
      {#if readout}<div class="readout">{readout}</div>{/if}
    </div>
  </section>
{/if}

<style>
  .syncview {
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
  label {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  input[type="number"] {
    width: 78px;
  }
  input.short {
    width: 48px;
  }
  .slider input {
    width: 110px;
  }
  .num {
    min-width: 6.5em;
    font-variant-numeric: tabular-nums;
  }
  .step {
    padding: 0 5px;
  }
  .presets,
  .seg {
    display: flex;
    gap: 4px;
  }
  .seg {
    gap: 0;
  }
  .seg button {
    padding: 1px 8px;
    border-radius: 0;
  }
  .seg button:first-child {
    border-radius: 3px 0 0 3px;
  }
  .seg button:last-child {
    border-radius: 0 3px 3px 0;
  }
  .seg button.on,
  .presets button.on {
    background: #ffd27a;
    color: #000;
  }
  .lock {
    font-variant-numeric: tabular-nums;
  }
  .lock.locked {
    color: #8fe08f;
  }
  .lock.hold {
    color: #ffd27a;
  }
  .levels {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
    padding: 0 8px 4px;
    color: var(--muted);
  }
  .legend {
    position: relative;
    width: 180px;
    height: 12px;
    border: 1px solid var(--line);
    border-radius: 2px;
    overflow: hidden;
  }
  .legend canvas {
    position: static;
    width: 100%;
    height: 100%;
    image-rendering: auto;
  }
  .legend .zero {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: #fff;
    mix-blend-mode: difference;
  }
  .screen {
    position: relative;
    height: min(55vh, 540px);
    margin: 0 8px 8px;
    background: #000;
    border: 1px solid var(--line);
    border-radius: 3px;
    overflow: hidden;
  }
  canvas {
    position: absolute;
    display: block;
  }
  .img {
    left: var(--gutter);
    top: 0;
    width: calc(100% - var(--gutter) - var(--band));
    height: 100%;
  }
  .overlay {
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }
  .wait,
  .readout {
    position: absolute;
    left: calc(var(--gutter) + 8px);
    color: rgba(255, 255, 255, 0.7);
    font-size: 11px;
    pointer-events: none;
  }
  .wait {
    top: 8px;
  }
  .readout {
    top: 6px;
    background: rgba(0, 0, 0, 0.65);
    padding: 1px 6px;
    border-radius: 3px;
  }
</style>
