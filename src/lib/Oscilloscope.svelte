<script lang="ts">
  // コンポジット信号のオシロスコープ (ライン選択付きの波形モニタ)。
  // 受像機に入る直前の信号 (帰線区間・同期・VBI のデータ込み) を、ライン番号で選んで CRT のビームで描く。
  // 表示の更新ごとに、前回より後に来たフィールドを全部受け取って蛍光面に積み重ねる (残光)。
  // 開いている間だけエンジンに信号を写してもらう。
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, untrack } from "svelte";
  import { CrtBeam } from "#lib/crtBeam.ts";
  import { scopeView } from "#lib/scopeView.svelte.ts";

  type Standard = "ntsc" | "pal";
  let { standard = "ntsc" as Standard } = $props();

  // ---- 表示設定 (localStorage に保存) ----
  type Span = "1" | "2" | "3" | "9" | "22" | "v";
  type Filter = "flat" | "lpass" | "chroma";
  /** analog = CRT ビーム (残光・輝度・フォーカス)、dso = デジタルオシロ風 (最新フィールドの細い線だけ) */
  type Mode = "analog" | "dso";
  type Saved = {
    open: boolean;
    mode: Mode;
    /** デコード画面に、見ているラインを重ねて表示する */
    overlay: boolean;
    line: Record<Standard, number>;
    span: Span;
    filter: Filter;
    /** 残光 [s] (見た目の明るさが 1/10 になるまで)。0 = なし、-1 = 無限 */
    persistence: number;
    fps: number;
    intensity: number;
    focus: number;
    /** DSO の線の太さ [CSS px] */
    dsoWidth: number;
  };
  const STORE_KEY = "vhs-oscilloscope";
  function load(): Saved {
    const d: Saved = {
      open: false,
      mode: "analog",
      overlay: true,
      line: { ntsc: 21, pal: 23 },
      span: "1",
      filter: "flat",
      persistence: 0.2,
      fps: 30,
      intensity: 1,
      focus: 1.3,
      dsoWidth: 1,
    };
    try {
      return { ...d, ...JSON.parse(localStorage.getItem(STORE_KEY) ?? "{}") };
    } catch {
      return d;
    }
  }
  const saved = load();
  let open = $state(saved.open);
  let mode = $state<Mode>(saved.mode === "dso" ? "dso" : "analog");
  let overlay = $state(saved.overlay !== false);
  let lines = $state(saved.line);
  let span = $state<Span>(saved.span);
  let filter = $state<Filter>(saved.filter);
  let persistence = $state(saved.persistence);
  let fps = $state(saved.fps);
  let intensity = $state(saved.intensity);
  let focus = $state(saved.focus);
  let dsoWidth = $state(saved.dsoWidth);
  $effect(() => {
    const s = JSON.stringify({ open, mode, overlay, line: lines, span, filter, persistence, fps, intensity, focus, dsoWidth });
    try {
      localStorage.setItem(STORE_KEY, s);
    } catch {}
  });

  const TOTAL: Record<Standard, number> = { ntsc: 525, pal: 625 };
  const FIELD_RATE: Record<Standard, number> = { ntsc: 60000 / 1001, pal: 50 };
  const LS: Record<Standard, number> = { ntsc: 910, pal: 1135 };
  const total = $derived(TOTAL[standard]);
  const line = $derived(Math.min(Math.max(1, lines[standard] ?? 1), TOTAL[standard]));
  function setLine(v: number) {
    const t = TOTAL[standard];
    const n = ((((Math.round(v) - 1) % t) + t) % t) + 1;
    lines = { ...lines, [standard]: n };
  }

  /** 表示するライン数 */
  const count = $derived(span === "v" ? (standard === "ntsc" ? 263 : 313) : Number(span));

  // デコード画面のオーバーレイへ、見ているラインを知らせる
  $effect(() => {
    scopeView.active = open && overlay;
    scopeView.standard = standard;
    scopeView.line = line;
    scopeView.count = count;
  });
  onDestroy(() => {
    scopeView.active = false;
  });

  // ---- 受け取った信号 ----
  type FieldRec = { id: number; pal: boolean; ls: number; fs: number; lines: { n: number; data: Float32Array }[] };
  let lastId = 0;
  let hasSignal = $state(false);
  /** ライン番号 → 最後に受け取った信号 (カーソルの読み取り用) */
  const lineCache = new Map<number, Float32Array>();
  let lastFs = 0;

  function parse(ab: ArrayBuffer): FieldRec[] {
    const out: FieldRec[] = [];
    if (ab.byteLength < 4) return out;
    const dv = new DataView(ab);
    let o = 0;
    const nf = dv.getUint32(o, true);
    o += 4;
    for (let f = 0; f < nf; f++) {
      const id = Number(dv.getBigUint64(o, true));
      const pal = dv.getUint32(o + 8, true) === 1;
      const ls = dv.getUint32(o + 16, true);
      const fs = dv.getFloat32(o + 20, true);
      const nl = dv.getUint32(o + 24, true);
      o += 28;
      const ls_: { n: number; data: Float32Array }[] = [];
      for (let k = 0; k < nl; k++) {
        const n = dv.getUint32(o, true);
        ls_.push({ n, data: new Float32Array(ab, o + 4, ls) });
        o += 4 + ls * 4;
      }
      out.push({ id, pal, ls, fs, lines: ls_ });
    }
    return out;
  }

  // ---- 縦軸 [IRE] ----
  const IRE_HI = 130;
  const IRE_LO = -50;
  const GUTTER = 44;
  const FOOT = 18;
  const yOf = (ire: number, h: number) => ((IRE_HI - ire) / (IRE_HI - IRE_LO)) * (h - FOOT);

  let beamCanvas: HTMLCanvasElement | undefined = $state();
  let gridCanvas: HTMLCanvasElement | undefined = $state();
  let dsoCanvas: HTMLCanvasElement | undefined = $state();
  let beam: CrtBeam | null = null;
  /** DSO で最後に描いたフィールド (表示サイズが変わったときの描き直し用) */
  let dsoLast: FieldRec | null = null;
  let glError = $state("");

  /** 表示の 1 サンプル (フィルタ込み)。LPASS は副搬送波 1 周期ぶんの平均で色を落とす */
  function filtered(d: Float32Array, k: number): number {
    const ls = d.length;
    const v = (i: number) => d[i < 0 ? 0 : i >= ls ? ls - 1 : i];
    if (filter === "flat") return v(k);
    const lp = 0.125 * v(k - 2) + 0.25 * (v(k - 1) + v(k) + v(k + 1)) + 0.125 * v(k + 2);
    return filter === "lpass" ? lp : v(k) - lp + 50;
  }

  // ---- ビームの線分を作る ----
  let segs = new Float32Array(5 * 65536);
  function grow(n: number) {
    if (n * 5 > segs.length) {
      const next = new Float32Array(Math.max(n * 5, segs.length * 2));
      next.set(segs);
      segs = next;
    }
  }

  /**
   * 1 フィールドぶんのビームの線分を作る。戻り値は線分の数。
   * 1 画素に何サンプルも入るときは、列ごとの最小・最大を往復する点にまとめ、その列のサンプル数ぶんのエネルギーを置く
   */
  function buildField(rec: FieldRec, energy: number, sc = beam?.scale ?? 1): number {
    if (!gridCanvas) return 0;
    const cssW = gridCanvas.clientWidth,
      cssH = gridCanvas.clientHeight;
    const x0 = GUTTER * sc;
    const pw = (cssW - GUTTER) * sc;
    const ph = cssH * sc;
    const ls = rec.ls;
    const totalSamples = count * ls;
    const yPx = (ire: number) => yOf(ire, cssH) * sc;
    // 1 ステップのエネルギー: 1 回の掃引で画面幅ぶんの光を置く (掃引の速さで明るさが変わらないように)
    const base = (energy * pw) / totalSamples;
    const perPx = totalSamples / pw;
    let n = 0;
    let px = NaN,
      py = NaN;
    const point = (x: number, y: number, e: number) => {
      if (!Number.isNaN(px)) {
        grow(n + 1);
        const o = n * 5;
        segs[o] = px;
        segs[o + 1] = py;
        segs[o + 2] = x;
        segs[o + 3] = y;
        segs[o + 4] = e;
        n++;
      }
      px = x;
      py = y;
    };
    let prevK = -2;
    for (const { n: num, data } of rec.lines) {
      const k = (num - line + total) % total;
      if (k >= count) continue;
      // 続いていないライン (このフィールドにない) の所ではビームを切る
      if (k !== prevK + 1) px = py = NaN;
      prevK = k;
      const start = k * ls;
      if (perPx <= 2) {
        for (let s = 0; s < ls; s++) {
          point(x0 + ((start + s) / totalSamples) * pw, yPx(filtered(data, s)), base);
        }
      } else {
        // 列ごとの最小・最大
        const c0 = Math.floor((start / totalSamples) * pw);
        const c1 = Math.ceil(((start + ls) / totalSamples) * pw);
        let up = true;
        for (let c = c0; c < c1; c++) {
          const s0 = Math.max(0, Math.floor((c / pw) * totalSamples) - start);
          const s1 = Math.min(ls, Math.floor(((c + 1) / pw) * totalSamples) - start);
          if (s1 <= s0) continue;
          let lo = Infinity,
            hi = -Infinity;
          for (let s = s0; s < s1; s++) {
            const v = filtered(data, s);
            if (v < lo) lo = v;
            if (v > hi) hi = v;
          }
          const e = (base * (s1 - s0)) / 2;
          const x = x0 + c + 0.5;
          if (up) {
            point(x, yPx(lo), e);
            point(x, yPx(hi), e);
          } else {
            point(x, yPx(hi), e);
            point(x, yPx(lo), e);
          }
          up = !up;
        }
      }
    }
    return n;
  }

  // ---- 取得と描画のループ ----
  let timer: ReturnType<typeof setTimeout> | null = null;
  let busy = false;
  let lastDraw = 0;

  async function tick() {
    timer = null;
    if (!open) return;
    const started = performance.now();
    if (!busy && (mode === "dso" ? dsoCanvas : beam)) {
      busy = true;
      try {
        const ls = LS[standard];
        // 残光があるときは、前回の更新から後のフィールドを全部受け取る (1 回に送る量は抑える)
        const between = Math.ceil(FIELD_RATE[standard] / Math.max(1, fps)) + 1;
        const budget = Math.max(1, Math.floor(2_400_000 / (count * ls)));
        // 最新の 1 フィールドだけにすると、片方のフィールドにしかないライン (21 など) は、更新の周期が
        // フィールドの周期の整数倍のときにいつも逆のフィールドを引いて止まる。2 フィールド取って新しい方から選ぶ
        const maxFields = mode === "dso" || persistence === 0 ? 2 : Math.min(between, budget, 16);
        const ab = await invoke<ArrayBuffer>("get_composite", { after: lastId, line, count, maxFields });
        const recs = parse(ab).filter((r) => (r.pal ? "pal" : "ntsc") === standard);
        if (mode === "dso") renderDso(recs);
        else render(recs);
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

  function render(recs: FieldRec[]) {
    if (!beam) return;
    if (beam.resize()) drawGrid();
    beam.sigma = focus;
    const now = performance.now();
    const dt = lastDraw ? Math.min(1, (now - lastDraw) / 1000) : 1 / fps;
    lastDraw = now;
    // 選んだラインの入っているフィールドだけが光る (ライン選択の掃引)
    let lit = ingest(recs);
    if (persistence === 0) lit = lit.slice(-1);
    if (lit.length) hasSignal = true;

    // 残光: 蛍光面を減衰させてから、届いたフィールドを足す。
    // 定常状態の明るさが更新の速さ・残光の長さによらないように、1 フィールドのエネルギーを (1 - 減衰) / 本数 にする
    let decay: number, gain: number;
    if (persistence === 0) {
      decay = lit.length ? 0 : 1;
      gain = 1;
    } else if (persistence < 0) {
      decay = 1;
      gain = 1 - Math.exp(-dt / 1.0);
    } else {
      // 残光の設定値は「見た目の明るさが 1/10 に落ちるまでの時間」。
      // 蛍光面の値は表示ガンマ (1/γ 乗) を通るので、見た目が 1/10 になるのは光量が 10^-γ になったとき。
      // 時定数をそのまま exp(-dt / 残光) に使うと、暗い尾が ln(10)·γ ≈ 5 倍長く見えてしまう
      const k = Math.LN10 * beam.gamma;
      decay = Math.exp((-dt * k) / persistence);
      gain = 1 - decay;
    }
    beam.fade(decay);
    for (const r of lit) {
      const n = buildField(r, (intensity * gain) / Math.max(1, lit.length));
      beam.drawBeam(segs, n);
    }
    beam.present();
  }

  /** 受け取ったフィールドの記録を取り込み、選んだラインの入っているものだけ返す */
  function ingest(recs: FieldRec[]): FieldRec[] {
    for (const r of recs) {
      // 番号が戻った (エンジンの作り直し) ときもそのまま追う
      lastId = r.id < lastId && r === recs[0] ? r.id : Math.max(lastId, r.id);
      lastFs = r.fs;
      for (const l of r.lines) lineCache.set(l.n, l.data);
    }
    return recs.filter((r) => r.lines.some((l) => (l.n - line + total) % total < count));
  }

  // ---- DSO 表示 (CRT エミュレーションなし): 最新のフィールドを細い線で描くだけ ----
  function renderDso(recs: FieldRec[]) {
    const lit = ingest(recs);
    if (lit.length) {
      hasSignal = true;
      dsoLast = lit[lit.length - 1];
    }
    drawDso();
  }

  function drawDso() {
    if (!dsoCanvas) return;
    const { ctx } = fitGrid(dsoCanvas);
    if (!dsoLast) return;
    const n = buildField(dsoLast, 0, 1);
    ctx.strokeStyle = "#ffe14a";
    ctx.lineWidth = dsoWidth;
    ctx.lineJoin = "round";
    ctx.lineCap = "round";
    ctx.beginPath();
    let ex = NaN,
      ey = NaN;
    for (let i = 0; i < n; i++) {
      const o = i * 5;
      // 線分の始点が前の終点と違えばビームが切れている所
      if (segs[o] !== ex || segs[o + 1] !== ey) ctx.moveTo(segs[o], segs[o + 1]);
      ex = segs[o + 2];
      ey = segs[o + 3];
      ctx.lineTo(ex, ey);
    }
    ctx.stroke();
  }

  function clearScreen() {
    beam?.clear();
    beam?.present();
    dsoLast = null;
    if (dsoCanvas) fitGrid(dsoCanvas);
  }

  // 開閉だけに反応する (tick が読む設定値を依存に入れると、設定を変えるたびにループが増える)
  $effect(() => {
    const active = open;
    untrack(() => {
      invoke("set_scope_active", { active }).catch(() => {});
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
      if (active) {
        lastId = 0;
        hasSignal = false;
        lastDraw = 0;
        timer = setTimeout(tick, 0);
      }
    });
  });

  // ビームの描画先 (アナログ表示で開いているときだけ作る)
  $effect(() => {
    if (!open || mode !== "analog" || !beamCanvas) return;
    try {
      beam = new CrtBeam(beamCanvas);
      beam.resize();
      glError = "";
    } catch (e) {
      glError = String(e);
      beam = null;
    }
    return () => {
      beam?.dispose();
      beam = null;
    };
  });

  onDestroy(() => {
    if (timer) clearTimeout(timer);
    invoke("set_scope_active", { active: false }).catch(() => {});
  });

  // 方式・ライン・範囲・フィルタを変えたら蛍光面を消す
  $effect(() => {
    void standard;
    void line;
    void span;
    void filter;
    void mode;
    lineCache.clear();
    untrack(clearScreen);
  });

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

  function fitGrid(c: HTMLCanvasElement) {
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

  function drawGrid() {
    if (!gridCanvas) return;
    const { ctx, w, h } = fitGrid(gridCanvas);
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
    ctx.fillText(filter === "chroma" ? "IRE (+50)" : "IRE", 4, 9);

    // 横軸
    const ls = LS[standard];
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
          const x = Math.round(xl(t * 1e-6 * fs)) + 0.5;
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
    void filter;
    if (!open || !gridCanvas) return;
    drawGrid();
    const ro = new ResizeObserver(() => {
      drawGrid();
      if (beam?.resize()) beam.present();
      if (mode === "dso") drawDso();
    });
    ro.observe(gridCanvas);
    return () => ro.disconnect();
  });

  // ---- カーソル読み取り ----
  let readout = $state("");
  function onMove(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    const r = el.getBoundingClientRect();
    const x = e.clientX - r.left - GUTTER;
    const y = e.clientY - r.top;
    const pw = r.width - GUTTER;
    if (x < 0 || x > pw || !lastFs) {
      readout = "";
      return;
    }
    const ls = LS[standard];
    const s = (x / pw) * count * ls;
    const l = Math.floor(s / ls);
    const k = s - l * ls;
    const num = ((line - 1 + l) % total) + 1;
    const ire = IRE_HI - (y / (r.height - FOOT)) * (IRE_HI - IRE_LO);
    const d = lineCache.get(num);
    const v = d ? filtered(d, Math.floor(k)) : NaN;
    readout =
      `ライン ${num} · ${((k / lastFs) * 1e6).toFixed(2)} µs · カーソル ${ire.toFixed(1)} IRE` +
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
      <span class="seg" role="group" aria-label="表示方式">
        <button class:on={mode === "analog"} onclick={() => (mode = "analog")} title="CRT のビームを再現 (残光・輝度・フォーカス)">アナログ</button>
        <button class:on={mode === "dso"} onclick={() => (mode = "dso")} title="デジタルオシロ風。最新のフィールドを細い線で描く (CRT エミュレーションなし)">DSO</button>
      </span>
      {#if mode === "analog"}
      <label title="蛍光面の残光 (見た目の明るさが 1/10 に落ちるまでの時間)">
        残光
        <select bind:value={persistence}>
          <option value={0}>なし</option>
          <option value={0.05}>0.05 s</option>
          <option value={0.2}>0.2 s</option>
          <option value={0.5}>0.5 s</option>
          <option value={1}>1 s</option>
          <option value={3}>3 s</option>
          <option value={-1}>無限</option>
        </select>
      </label>
      <label title="表示の更新の速さ。残光があれば間のフィールドもすべて積み重ねる">
        更新
        <select bind:value={fps}>
          <option value={5}>5 fps</option>
          <option value={10}>10 fps</option>
          <option value={15}>15 fps</option>
          <option value={20}>20 fps</option>
          <option value={30}>30 fps</option>
          <option value={60}>60 fps</option>
        </select>
      </label>
      <label class="slider" title="ビームの明るさ (ダブルクリックで既定値)">
        輝度
        <input
          type="range"
          min="-3"
          max="3"
          step="0.05"
          value={Math.log2(intensity)}
          oninput={(e) => (intensity = 2 ** Number((e.target as HTMLInputElement).value))}
          ondblclick={() => (intensity = 1)}
        />
      </label>
      <label class="slider" title="ビームの太さ (ダブルクリックで既定値)">
        フォーカス
        <input
          type="range"
          min="0.5"
          max="4"
          step="0.05"
          bind:value={focus}
          ondblclick={() => (focus = 1.3)}
        />
      </label>
      {:else}
      <label class="slider" title="線の太さ (ダブルクリックで既定値)">
        線幅
        <input
          type="range"
          min="0.5"
          max="5"
          step="0.25"
          bind:value={dsoWidth}
          oninput={() => drawDso()}
          ondblclick={() => (dsoWidth = 1)}
        />
        <span class="num">{dsoWidth.toFixed(2)} px</span>
      </label>
      {/if}
      <span class="presets">
        <button onclick={() => { setLine(standard === "ntsc" ? 1 : 309); span = "22"; }} title="垂直帰線区間">V 帰線</button>
        {#if standard === "ntsc"}
          <button onclick={() => { setLine(21); span = "1"; }} title="ライン 21 の字幕">字幕</button>
        {/if}
        <button onclick={() => { setLine(17); span = "2"; }} title="挿入試験信号">VITS</button>
        <button onclick={clearScreen} title="表示を消す">消去</button>
        <button class:on={overlay} onclick={() => (overlay = !overlay)} title="見ているラインをデコード画面に重ねて表示する">画面に表示</button>
      </span>
    {/if}
  </header>
  {#if open}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="screen" style:--gutter="{GUTTER}px" onpointermove={onMove} onpointerleave={() => (readout = "")} onwheel={onWheel}>
      <canvas class="grid" bind:this={gridCanvas}></canvas>
      {#if mode === "analog"}
        <canvas class="beam" bind:this={beamCanvas}></canvas>
      {:else}
        <canvas class="dso" bind:this={dsoCanvas}></canvas>
      {/if}
      {#if glError}
        <div class="wait">ビームを描けません: {glError}</div>
      {:else if !hasSignal}
        <div class="wait">信号を待っています… (コンポジット信号を通しているときだけ表示)</div>
      {/if}
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
  .slider input {
    width: 80px;
  }
  .num {
    min-width: 4.2em;
    font-variant-numeric: tabular-nums;
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
  .seg {
    display: flex;
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
  .screen {
    position: relative;
    height: 260px;
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
    pointer-events: none;
  }
  .beam {
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
