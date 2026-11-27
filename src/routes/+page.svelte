<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { onMount, untrack } from "svelte";
  import { parseQuery, search, segments, type SearchResult } from "#lib/search.ts";
  import { groups, help, getPath, setPath, isActive, presets, applyDiff, retarget, type Params } from "#lib/params.ts";
  import Scopes from "#lib/Scopes.svelte";
  import Oscilloscope from "#lib/Oscilloscope.svelte";
  import SyncView from "#lib/SyncView.svelte";
  import ScopeOverlay from "#lib/ScopeOverlay.svelte";
  import Immersive from "#lib/Immersive.svelte";
  import Menu from "#lib/Menu.svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  type Stats = {
    source: string;
    fields_per_sec: number;
    camera_ms: number;
    encode_ms: number;
    tape_ms: number;
    rf_ms: number;
    decode_ms: number;
    capture_ms: number;
    crt_ms: number;
    crt_version: number;
    codec_q: number | null;
    codec_kbit: number | null;
    recording: string | null;
    error: string | null;
  };
  type ShaderParam = { id: string; description: string; initial: number; min: number; max: number; step: number };
  type CrtInfo = { preset: string; loading: boolean; error: string | null; gpu: string | null; params: ShaderParam[] };
  type ExportProgress = { done: number; total: number | null; finished: boolean; error: string | null };
  type PerformanceSegment = { key: string; label: string; ms: number };
  type WindowInfo = { title: string; hwnd: number; width: number; height: number };

  let params = $state<Params | null>(null);
  let defaults = $state.raw<Params | null>(null);
  let stats = $state<Stats | null>(null);
  let cameras = $state<string[]>([]);
  let camera = $state("");
  let cameraScaling = $state("area");
  let windowTitle = $state("");
  let windowHwnd = $state<number | null>(null);
  let windowAudio = $state(true);
  let windowMonitor = $state(true);
  let windows = $state<WindowInfo[]>([]);
  let windowsLoading = $state(false);
  /** false ならウィンドウ全体をキャプチャする */
  let windowCrop = $state(false);
  let windowCropX = $state(0);
  let windowCropY = $state(0);
  let windowCropWidth = $state(1280);
  let windowCropHeight = $state(720);
  let windowScaling = $state("area");
  const PATTERNS = [
    { kind: "pattern", label: "診断パターン" },
    { kind: "smpte", label: "SMPTE カラーバー" },
    { kind: "ebu", label: "EBU カラーバー" },
  ];
  const SCALINGS = [
    { value: "nearest", label: "Nearest" },
    { value: "bilinear", label: "Bilinear" },
    { value: "bicubic", label: "Bicubic" },
    { value: "area", label: "Area" },
    { value: "lanczos", label: "Lanczos" },
  ];
  let message = $state("");
  let exporting = $state<ExportProgress | null>(null);
  let canvas: HTMLCanvasElement;
  /** オシロスコープのオーバーレイが重ねる先 (マウント後に入る) */
  let canvasEl = $state<HTMLCanvasElement>();
  let scopes: Scopes | undefined = $state();
  // 画面の下のパネル (タブで 1 つだけ開く。開いているタブを押すと閉じる)
  type PanelTab = "scopes" | "osc" | "sync";
  const PANEL_TABS: { id: PanelTab; label: string; title: string }[] = [
    { id: "scopes", label: "スコープ", title: "映像波形とベクトルスコープ" },
    { id: "osc", label: "オシロスコープ", title: "コンポジット信号をライン番号で選んで波形で見る" },
    { id: "sync", label: "生信号", title: "コンポジット信号をデコードせずに白黒の画像で見る (帰線区間・同期も含めて全部。垂直同期で揃える)" },
  ];
  const PANEL_KEY = "vhs-panel-tab";
  let panelTab = $state<PanelTab | null>(
    (() => {
      try {
        const v = localStorage.getItem(PANEL_KEY);
        return PANEL_TABS.some((t) => t.id === v) ? (v as PanelTab) : null;
      } catch {
        return null;
      }
    })(),
  );
  $effect(() => {
    try {
      if (panelTab) localStorage.setItem(PANEL_KEY, panelTab);
      else localStorage.removeItem(PANEL_KEY);
    } catch {}
  });
  let crtInfo = $state<CrtInfo | null>(null);
  let crtVersion = -1;
  let crtFilter = $state("");

  // ---- パラメータ検索 ----
  // 検索中は一致したグループを自動で開き、一致しないグループは閉じる。
  // 検索をやめると、検索前に手で開閉していた状態に戻す。
  let paramQuery = $state("");
  let searchBox = $state<HTMLInputElement | null>(null);
  let tokens = $derived(parseQuery(paramQuery));
  let searching = $derived(tokens.length > 0);
  /** 検索していないときの開閉状態 (キー: "g0".., "crt", "crt-params") */
  let userOpen = $state<Record<string, boolean>>(Object.fromEntries(groups.map((_, i) => [`g${i}`, i < 2])));
  /** 検索中に手で開閉した分。検索語が変わったら捨てて自動の開閉に戻す */
  let searchOpen = $state<Record<string, boolean>>({});

  const optionLabels = (c: (typeof groups)[number]["controls"][number]) =>
    c.kind === "select" ? c.options.map((o) => o.label) : [];

  /** グループごとの一致 (path → 結果) */
  let groupHits = $derived.by(() =>
    groups.map((g) => {
      const hits = new Map<string, SearchResult>();
      if (!searching) return hits;
      for (const c of g.controls) {
        const r = search(tokens, {
          label: c.label,
          keys: [g.title, c.path],
          text: [help[c.path] ?? "", ...optionLabels(c)],
        });
        if (r) hits.set(c.path, r);
      }
      return hits;
    }),
  );

  // CRT セクションの固定の行
  const CRT_TITLE = "ブラウン管 (CRT シェーダー)";
  const CRT_ROWS = [
    { key: "enabled", label: "シェーダーを掛ける", text: "retroarch slang slangp" },
    { key: "preset", label: "プリセット", text: "slangp crt-royale" },
    { key: "height", label: "録画・書き出しの高さ", text: "解像度" },
  ];
  let crtHits = $derived.by(() => {
    const rows = new Map<string, SearchResult>();
    const ps = new Map<string, SearchResult>();
    if (!searching) return { rows, params: ps };
    for (const r of CRT_ROWS) {
      const h = search(tokens, { label: r.label, keys: [CRT_TITLE], text: [r.text] });
      if (h) rows.set(r.key, h);
    }
    const shader = crtInfo && params && crtInfo.preset === params.crt.preset ? crtInfo.params : [];
    for (const p of shader) {
      if (p.min === p.max) continue;
      const h = search(tokens, { label: p.description, keys: ["シェーダーパラメータ", p.id] });
      if (h) ps.set(p.id, h);
    }
    return { rows, params: ps };
  });
  let matchCount = $derived(groupHits.reduce((n, h) => n + h.size, 0) + crtHits.rows.size + crtHits.params.size);

  function isOpen(key: string, auto: boolean) {
    return searching ? (searchOpen[key] ?? auto) : (userOpen[key] ?? false);
  }
  function onToggle(key: string, e: Event) {
    const o = (e.currentTarget as HTMLDetailsElement).open;
    if (searching) searchOpen[key] = o;
    else userOpen[key] = o;
  }
  function onSearchInput() {
    searchOpen = {};
  }
  function onSearchKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (paramQuery) {
        paramQuery = "";
        searchOpen = {};
      } else searchBox?.blur();
      e.preventDefault();
    } else if (e.key === "Enter") {
      // 最初に一致したコントロールへ移る
      e.preventDefault();
      const el = document.querySelector<HTMLElement>(
        ".panel .hit :is(input:not([type=search]), select, textarea, button)",
      );
      el?.scrollIntoView({ block: "center" });
      el?.focus();
    }
  }
  /** 検索中に表示するか (一致しないグループを手で開いたときは全部見せる) */
  function shown(hits: Map<string, unknown>, key: string) {
    return !searching || hits.size === 0 || hits.has(key);
  }

  // CRT を有効にしたら (プリセットの読込なども含め) セクションを開き、無効にしたら閉じる
  $effect(() => {
    const en = !!params?.crt.enabled;
    untrack(() => (userOpen.crt = en));
  });
  const CRT_HEIGHTS = [720, 960, 1080, 1440, 2160];
  const FIELD_RATES: Record<Params["standard"], number> = { ntsc: 60000 / 1001, pal: 50 };
  let fieldBudgetMs = $derived(1000 / FIELD_RATES[params?.standard ?? "ntsc"]);
  let performanceSegments = $derived.by((): PerformanceSegment[] => {
    if (!stats) return [];
    return [
      { key: "camera", label: "入力・カメラ", ms: stats.camera_ms },
      { key: "encode", label: "エンコード", ms: stats.encode_ms },
      { key: "tape", label: "テープ", ms: stats.tape_ms },
      { key: "rf", label: "RF", ms: stats.rf_ms },
      { key: "decode", label: "デコード", ms: stats.decode_ms },
      { key: "capture", label: "キャプチャ", ms: stats.capture_ms },
      { key: "crt", label: "CRT", ms: stats.crt_ms },
    ].filter((segment) => Number.isFinite(segment.ms) && segment.ms > 0);
  });
  let performanceTotalMs = $derived(performanceSegments.reduce((sum, segment) => sum + segment.ms, 0));
  let performanceOverBudget = $derived(performanceTotalMs > fieldBudgetMs);

  const VIDEO_EXTS = ["mp4", "mkv", "mov", "avi", "webm", "m4v", "mpg", "ts"];
  const IMAGE_EXTS = ["png", "jpg", "jpeg", "bmp", "webp", "exr"];

  async function run(f: () => Promise<unknown>) {
    try {
      message = "";
      await f();
    } catch (e) {
      message = String(e);
    }
  }

  // パラメータが変わるたびにエンジンへ送る
  $effect(() => {
    if (!params) return;
    const snap = $state.snapshot(params);
    invoke("set_params", { params: snap });
  });

  async function refreshCameras() {
    cameras = await invoke<string[]>("list_cameras");
    if (!cameras.includes(camera)) {
      camera = cameras.find((c) => c.includes("OBS Virtual Camera")) ?? cameras[0] ?? "";
    }
  }

  function openCamera() {
    run(() => invoke("open_source", { req: { kind: "camera", device: camera, scaling: cameraScaling } }));
  }

  async function refreshWindows() {
    windowsLoading = true;
    try {
      windows = await invoke<WindowInfo[]>("list_windows");
    } catch (e) {
      message = String(e);
    } finally {
      windowsLoading = false;
    }
  }

  function openWindow(title = windowTitle, hwnd = windowHwnd) {
    if (!title) {
      message = "キャプチャするウィンドウを選んでください";
      return;
    }
    if (windowCrop && (windowCropX < 0 || windowCropY < 0 || windowCropWidth < 1 || windowCropHeight < 1)) {
      message = "クロップ位置は 0 以上、幅と高さは 1 以上にしてください";
      return;
    }
    windowTitle = title;
    windowHwnd = hwnd;
    // 幅・高さ 0 はクロップなし (ウィンドウ全体)
    const crop = windowCrop
      ? { crop_x: windowCropX, crop_y: windowCropY, crop_width: windowCropWidth, crop_height: windowCropHeight }
      : { crop_x: 0, crop_y: 0, crop_width: 0, crop_height: 0 };
    run(() =>
      invoke("open_source", {
        req: {
          kind: "window",
          title,
          hwnd,
          capture_audio: windowAudio,
          monitor_audio: windowMonitor,
          ...crop,
          scaling: windowScaling,
        },
      }),
    );
  }

  function openPattern(kind: string) {
    run(() => invoke("open_source", { req: { kind } }));
  }

  async function openFile() {
    const path = await open({
      filters: [
        { name: "動画・画像", extensions: [...VIDEO_EXTS, ...IMAGE_EXTS] },
        { name: "すべて", extensions: ["*"] },
      ],
    });
    if (typeof path === "string") run(() => invoke("open_source", { req: { kind: "file", path } }));
  }

  async function toggleRecording() {
    if (stats?.recording) {
      run(() => invoke("stop_recording"));
      return;
    }
    const path = await save({ defaultPath: "vhs_capture.mp4", filters: [{ name: "MP4", extensions: ["mp4"] }] });
    if (path) run(() => invoke("start_recording", { path }));
  }

  async function startExport() {
    const input = await open({ filters: [{ name: "動画", extensions: VIDEO_EXTS }] });
    if (typeof input !== "string") return;
    const output = await save({
      defaultPath: input.replace(/\.[^.\\/]+$/, "") + "_vhs.mp4",
      filters: [
        { name: "MP4", extensions: ["mp4"] },
        // コンポジット信号の raw サンプル列 (FL2000 + osmo-fl2k などで実機のテレビへ)
        { name: "コンポジット raw (fl2k_file 用 符号付き 8 bit)", extensions: ["s8"] },
        { name: "コンポジット raw (符号なし 8 bit)", extensions: ["u8"] },
        { name: "コンポジット raw (16 bit, 4fsc)", extensions: ["s16"] },
      ],
    });
    if (!output) return;
    exporting = { done: 0, total: null, finished: false, error: null };
    run(() => invoke("start_export", { input, output }));
  }

  async function savePreset() {
    const path = await save({ defaultPath: "preset.json", filters: [{ name: "JSON", extensions: ["json"] }] });
    if (path && params) run(() => invoke("save_preset", { path, params: $state.snapshot(params) }));
  }

  async function loadPreset() {
    const path = await open({ filters: [{ name: "JSON", extensions: ["json"] }] });
    if (typeof path === "string") {
      run(async () => {
        params = await invoke<Params>("load_preset", { path });
      });
    }
  }

  async function pickCrtPreset() {
    if (!params) return;
    const cur = params.crt.preset;
    const file = await open({
      defaultPath: cur ? cur.replace(/[\\/][^\\/]*$/, "") : undefined,
      filters: [
        { name: "slang プリセット", extensions: ["slangp"] },
        { name: "すべて", extensions: ["*"] },
      ],
    });
    if (typeof file === "string") {
      // 別のプリセットにはパラメータの上書きを持ち越さない
      if (file !== params.crt.preset) params.crt.params = {};
      params.crt.preset = file;
      params.crt.enabled = true;
    }
  }

  function crtValue(p: ShaderParam) {
    return params?.crt.params[p.id] ?? p.initial;
  }

  function setCrtValue(p: ShaderParam, v: number | null) {
    if (!params) return;
    if (v === null || Math.abs(v - p.initial) < 1e-6) delete params.crt.params[p.id];
    else params.crt.params[p.id] = v;
  }

  // 今のプリセットのパラメータ (min = max は見出し)。絞り込み中は見出しを省く
  let crtVisible = $derived.by(() => {
    const ps = crtInfo && params && crtInfo.preset === params.crt.preset ? crtInfo.params : [];
    const q = crtFilter.trim().toLowerCase();
    // パネル全体の検索でシェーダーパラメータに一致があればそれだけを出す
    const global = crtHits.params;
    if (!q && global.size === 0) return ps;
    return ps.filter(
      (p) =>
        p.min !== p.max &&
        (!q || p.id.toLowerCase().includes(q) || p.description.toLowerCase().includes(q)) &&
        (global.size === 0 || global.has(p.id)),
    );
  });

  function applyPreset(i: number) {
    // 今のテレビ方式とテープのフォーマットは引き継ぐ (プリセットが指定していればそちら)
    const preset = presets[i];
    if (!defaults || !preset) return;
    // FM モデルで使っていたときだけフォーマットの値をすべて引き継ぐ (簡易モデルの既定の見た目は変えない)
    const fm = params?.tape.luma_model === "fm";
    const standard = preset.diff.standard ?? params?.standard ?? "ntsc";
    const format = preset.diff.tape?.format ?? params?.tape.format ?? "vhs";
    // 形式を切り替えるプリセットでは、その形式に対応する搬送波・帯域などを先に適用する。
    const base = retarget(defaults, standard, format, preset.diff.tape?.format !== undefined || fm);
    const crt = params ? $state.snapshot(params.crt) : null;
    const image = params ? $state.snapshot(params.image) : null;
    params = applyDiff(base, preset.diff);
    // ブラウン管の設定は表示環境、画像の配置は入力の設定なので VHS のプリセットでは変えない
    if (crt && params) params.crt = crt;
    if (image && params) params.image = image;
  }

  async function pickFont(path: string) {
    const file = await open({
      filters: [
        { name: "フォント", extensions: ["ttf", "otf", "ttc", "otc"] },
        { name: "すべて", extensions: ["*"] },
      ],
    });
    if (typeof file === "string" && params) setPath(params, path, file);
  }

  // 選んだフォントが読めるか確認して名前を表示する
  let fontStatus = $state<{ text: string; error: boolean } | null>(null);
  $effect(() => {
    const font = params?.osd.font ?? "";
    const index = params?.osd.font_index ?? 0;
    if (!font) {
      fontStatus = null;
      return;
    }
    let stale = false;
    invoke<string>("check_font", { path: font, index })
      .then((name) => !stale && (fontStatus = { text: `読み込み OK: ${name}`, error: false }))
      .catch((e) => !stale && (fontStatus = { text: `${e} (内蔵フォントで表示中)`, error: true }));
    return () => {
      stale = true;
    };
  });

  /** 値を変える。方式・フォーマットの切り替えは、それに合う値 (帯域・搬送波・電源周波数など) もまとめて合わせる */
  function setControl(path: string, v: unknown) {
    if (!params) return;
    if (path === "standard" || path === "tape.format") {
      // $state のプロキシは structuredClone できないので、スナップショットを渡す
      const snap = $state.snapshot(params) as Params;
      setPath(snap, path, v);
      params = retarget(snap, snap.standard, snap.tape.format, path === "tape.format");
    } else setPath(params, path, v);
  }

  // イマーシブモード: UI を隠して全画面にし、デッキのメニュー表示 (映像信号に重畳) で操作する
  let immersive = $state(false);
  async function setImmersive(on: boolean) {
    immersive = on;
    try {
      await getCurrentWindow().setFullscreen(on);
    } catch (e) {
      console.error(e);
    }
  }

  function onGlobalKey(e: KeyboardEvent) {
    // イマーシブ中のキーは Immersive が受ける
    if (!immersive && e.key === "F11") {
      e.preventDefault();
      setImmersive(true);
    } else if (!immersive && (e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "f") {
      e.preventDefault();
      searchBox?.focus();
      searchBox?.select();
    }
  }

  function newSeed() {
    if (params) params.seed = Math.floor(Math.random() * 2 ** 31);
  }

  onMount(() => {
    let alive = true;
    let lastId = 0;
    const ctx = canvas.getContext("2d")!;
    canvasEl = canvas;

    // 新しいフレームがあれば取得して描画する (描画周期に合わせたプル型)
    const loop = async () => {
      while (alive) {
        try {
          const buf = await invoke<ArrayBuffer>("get_frame", { after: lastId });
          if (buf.byteLength > 16) {
            const view = new DataView(buf);
            lastId = Number(view.getBigUint64(0, true));
            const w = view.getUint32(8, true);
            const h = view.getUint32(12, true);
            if (canvas.width !== w || canvas.height !== h) {
              canvas.width = w;
              canvas.height = h;
            }
            const px = new Uint8ClampedArray(buf, 16, w * h * 4);
            ctx.putImageData(new ImageData(px, w, h), 0, 0);
            // CRT シェーダーを掛けたときは元の画像が後ろに付いている。スコープは元の画像で見る
            const rawAt = 16 + w * h * 4;
            if (buf.byteLength > rawAt + 8) {
              const rw = view.getUint32(rawAt, true);
              const rh = view.getUint32(rawAt + 4, true);
              scopes?.push(new Uint8ClampedArray(buf, rawAt + 8, rw * rh * 4), rw, rh);
            } else {
              scopes?.push(px, w, h);
            }
          }
        } catch (e) {
          console.error(e);
        }
        await new Promise((r) => requestAnimationFrame(r));
      }
    };
    loop();

    const statTimer = setInterval(async () => {
      stats = await invoke<Stats>("get_stats");
      if (stats.crt_version !== crtVersion) {
        crtVersion = stats.crt_version;
        crtInfo = await invoke<CrtInfo>("get_crt_info");
      }
    }, 500);

    // CRT シェーダーは画面上の実画素数で描く (マスクやスキャンラインを拡大縮小でつぶさないため)
    let lastVh = 0;
    const ro = new ResizeObserver(() => {
      const vh = Math.round(canvas.getBoundingClientRect().height * devicePixelRatio);
      if (vh > 0 && vh !== lastVh) {
        lastVh = vh;
        invoke("set_viewport", { height: vh });
      }
    });
    ro.observe(canvas);

    const unlisten = listen<ExportProgress>("export-progress", (e) => {
      exporting = e.payload;
      if (e.payload.finished) {
        message = e.payload.error ? `書き出し失敗: ${e.payload.error}` : "書き出しが完了しました";
        exporting = null;
      }
    });

    (async () => {
      defaults = await invoke<Params>("default_params");
      params = await invoke<Params>("get_params");
      await refreshCameras().catch((e) => (message = String(e)));
    })();

    return () => {
      alive = false;
      clearInterval(statTimer);
      ro.disconnect();
      unlisten.then((f) => f());
    };
  });

  function fmt(v: number, step: number) {
    const d = step >= 1 ? 0 : Math.min(4, Math.ceil(-Math.log10(step)));
    return v.toFixed(d);
  }
</script>

<svelte:window onkeydown={onGlobalKey} />

{#snippet label(text: string, hit: SearchResult | undefined)}
  {#each segments(text, hit?.labelHits ?? []) as seg}{#if seg.hit}<mark>{seg.t}</mark>{:else}{seg.t}{/if}{/each}
{/snippet}

{#if immersive && params}
  <Immersive
    {params}
    {defaults}
    source={stats?.source ?? ""}
    recording={!!stats?.recording}
    {message}
    {cameras}
    bind:camera
    actions={{
      openPattern: () => openPattern("pattern"),
      openSmpte: () => openPattern("smpte"),
      openEbu: () => openPattern("ebu"),
      openCamera,
      openFile,
      refreshCameras: () => run(refreshCameras),
      toggleRecording,
      applyPreset,
      setControl,
      exit: () => setImmersive(false),
    }}
  />
{/if}

<main class:immersive>
  <section class="stage">
    <div class="toolbar">
      <Menu label="テストパターン" title="テストパターンを表示する">
        {#snippet children(close)}
          {#each PATTERNS as p (p.kind)}
            <button class="item" onclick={() => { close(); openPattern(p.kind); }}>{p.label}</button>
          {/each}
        {/snippet}
      </Menu>
      <Menu label="カメラ" title="カメラ (DirectShow デバイス) を開く">
        {#snippet children(close)}
          <div class="head">
            カメラ
            <button class="small" onclick={() => run(refreshCameras)} title="カメラ一覧を更新">⟳</button>
          </div>
          <div class="list">
            {#each cameras as c (c)}
              <button class="item" class:current={c === camera} title={c} onclick={() => { camera = c; close(); openCamera(); }}>{c}</button>
            {:else}
              <div class="empty">カメラが見つかりません</div>
            {/each}
          </div>
          <div class="sep"></div>
          <label class="field">補間方式
            <select bind:value={cameraScaling} title="カメラのスケーリング方式">
              {#each SCALINGS as m (m.value)}<option value={m.value}>{m.label}</option>{/each}
            </select>
          </label>
        {/snippet}
      </Menu>
      <Menu label="ウィンドウ" title="ウィンドウをキャプチャする" onopen={refreshWindows}>
        {#snippet children(close)}
          <div class="head">
            {windowsLoading ? "ウィンドウを探しています…" : "ウィンドウ"}
            <button class="small" onclick={refreshWindows} title="ウィンドウ一覧を更新">⟳</button>
          </div>
          <div class="list">
            {#each windows as w, i (i)}
              <button class="item" class:current={w.title === windowTitle} title={w.title} onclick={() => { close(); openWindow(w.title, w.hwnd); }}>
                <span class="title">{w.title}</span><span class="sub">{w.width}×{w.height}</span>
              </button>
            {:else}
              {#if !windowsLoading}<div class="empty">キャプチャできるウィンドウがありません</div>{/if}
            {/each}
          </div>
          <div class="sep"></div>
          <label class="field" title="ウィンドウのアプリ (と子プロセス) の音声を取り込み、テープの音声処理に通して録画にも入れる">
            <input type="checkbox" bind:checked={windowAudio} /> 音声も取り込む
          </label>
          {#if windowAudio}
            <label class="field" title="処理した音声を既定の出力デバイスで鳴らす。元の音と重なるときは、アプリの出力先を別のデバイスにするか、こちらを切る">
              <input type="checkbox" bind:checked={windowMonitor} /> 処理した音をスピーカーで鳴らす
            </label>
          {/if}
          <label class="field"><input type="checkbox" bind:checked={windowCrop} /> 範囲を切り抜く</label>
          {#if windowCrop}
            <div class="field crop">
              <label>X <input class="crop-num" type="number" min="0" bind:value={windowCropX} /></label>
              <label>Y <input class="crop-num" type="number" min="0" bind:value={windowCropY} /></label>
              <label>幅 <input class="crop-num" type="number" min="1" bind:value={windowCropWidth} /></label>
              <label>高さ <input class="crop-num" type="number" min="1" bind:value={windowCropHeight} /></label>
            </div>
          {/if}
          <label class="field">補間方式
            <select bind:value={windowScaling} title="ウィンドウキャプチャのスケーリング方式">
              {#each SCALINGS as m (m.value)}<option value={m.value}>{m.label}</option>{/each}
            </select>
          </label>
          {#if windowTitle}
            <button class="item" onclick={() => { close(); openWindow(); }} title="今の設定で開き直す">
              「{windowTitle}」を開き直す
            </button>
          {/if}
        {/snippet}
      </Menu>
      <button onclick={openFile}>ファイルを開く…</button>
      <span class="spacer"></span>
      <button class:rec={!!stats?.recording} onclick={toggleRecording}>
        {stats?.recording ? "■ 録画停止" : "● 録画"}
      </button>
      <button onclick={startExport} disabled={!!exporting}>動画を書き出し…</button>
      <button onclick={() => setImmersive(true)} title="UI を隠して全画面にし、デッキのメニュー表示 (OSD) で操作する (F11)">イマーシブ</button>
    </div>

    <div class="screen">
      <canvas bind:this={canvas} width="754" height="480"></canvas>
      <ScopeOverlay target={canvasEl} hidden={immersive} />
    </div>

    <div class="panels">
      <div class="tabs" role="tablist">
        {#each PANEL_TABS as t (t.id)}
          <button
            role="tab"
            class:on={panelTab === t.id}
            aria-selected={panelTab === t.id}
            title={t.title}
            onclick={() => (panelTab = panelTab === t.id ? null : t.id)}>{t.label}</button
          >
        {/each}
        {#if panelTab}
          <button class="tab-close" onclick={() => (panelTab = null)} title="パネルを閉じる">▾</button>
        {/if}
      </div>
      <Scopes bind:this={scopes} standard={params?.standard ?? "ntsc"} open={panelTab === "scopes"} />
      <Oscilloscope standard={params?.standard ?? "ntsc"} open={panelTab === "osc"} />
      <SyncView standard={params?.standard ?? "ntsc"} open={panelTab === "sync"} />
    </div>

    <div class="status">
      <span>{stats?.source ?? "…"}</span>
      {#if stats}
        <span>{stats.fields_per_sec.toFixed(1)} fields/s</span>
        <div class="perf-display">
          <span class="perf-summary">
            処理 {(stats.camera_ms + stats.encode_ms + stats.tape_ms + stats.rf_ms + stats.decode_ms + stats.capture_ms).toFixed(2)} ms
            ({stats.camera_ms > 0.01 ? `cam ${stats.camera_ms.toFixed(2)} / ` : ""}enc {stats.encode_ms.toFixed(2)} / tape {stats.tape_ms.toFixed(2)}{stats.rf_ms > 0.01
              ? ` / rf ${stats.rf_ms.toFixed(2)}`
              : ""} / dec {stats.decode_ms.toFixed(2)}{stats.capture_ms > 0.01 ? ` / cap ${stats.capture_ms.toFixed(2)}` : ""}{stats.crt_ms > 0.01
              ? ` / crt ${stats.crt_ms.toFixed(2)}`
              : ""})
          </span>
          <div
            class="perf-chart"
            title={`各区間は1フィールドの処理時間。上限は方式から換算した1フィールド分 (${fieldBudgetMs.toFixed(2)} ms)。`}
            role="img"
            aria-label={`1フィールドの処理時間 ${performanceTotalMs.toFixed(2)} ミリ秒、上限 ${fieldBudgetMs.toFixed(2)} ミリ秒`}
          >
            <div class="perf-track" class:over-budget={performanceOverBudget}>
              {#each performanceSegments as segment (segment.key)}
                <span
                  class={`perf-segment perf-${segment.key}`}
                  style={`flex: 0 0 ${(segment.ms / fieldBudgetMs) * 100}%`}
                  title={`${segment.label}: ${segment.ms.toFixed(2)} ms`}
                  aria-hidden="true"
                ></span>
              {/each}
            </div>
            <span class="perf-total" class:over-budget-text={performanceOverBudget}>
              {performanceTotalMs.toFixed(2)} / {fieldBudgetMs.toFixed(2)} ms
            </span>
          </div>
        </div>
        {#if stats.codec_q != null && stats.codec_kbit != null}
          <span title="キャプチャ圧縮の量子化スケールと直前フレームの符号量">
            q {stats.codec_q.toFixed(1)} · {stats.codec_kbit.toFixed(0)} kbit/frame
          </span>
        {/if}
        {#if stats.recording}<span class="rec-label">録画中: {stats.recording}</span>{/if}
        {#if stats.error}<span class="err">{stats.error}</span>{/if}
      {/if}
      {#if exporting}
        <span>
          書き出し中 {exporting.done}{exporting.total ? ` / ${exporting.total}` : ""} fields
          {#if exporting.total}({((exporting.done / exporting.total) * 100).toFixed(0)}%){/if}
        </span>
        <button class="small" onclick={() => invoke("cancel_export")}>中止</button>
      {/if}
      {#if message}<span class="err">{message}</span>{/if}
    </div>
  </section>

  <aside class="panel">
    <div class="presets">
      <select onchange={(e) => applyPreset(Number((e.target as HTMLSelectElement).value))}>
        <option value="" disabled selected>プリセット…</option>
        {#each presets as p, i}<option value={i}>{p.name}</option>{/each}
      </select>
      <button onclick={loadPreset}>読込</button>
      <button onclick={savePreset}>保存</button>
    </div>
    {#if params}
      <div class="search">
        <input
          type="search"
          placeholder="パラメータを検索 (Ctrl+F)"
          bind:this={searchBox}
          bind:value={paramQuery}
          oninput={onSearchInput}
          onkeydown={onSearchKey}
          title="名前・説明・パスで曖昧検索。空白区切りで AND。Enter で最初の一致へ、Esc で消去"
        />
        {#if searching}<span class="count" class:none={matchCount === 0}>{matchCount} 件</span>{/if}
      </div>
      <div class="seed">
        <label for="seed">シード</label>
        <input id="seed" type="number" bind:value={params.seed} />
        <button onclick={newSeed}>🎲</button>
      </div>
      {#each groups as g, gi}
        {@const hits = groupHits[gi]}
        <details
          open={isOpen(`g${gi}`, hits.size > 0)}
          ontoggle={(e) => onToggle(`g${gi}`, e)}
          class:nomatch={searching && hits.size === 0}
        >
          <summary>{g.title}{#if searching && hits.size > 0}<span class="hits">{hits.size}</span>{/if}</summary>
          {#if g.note && !(searching && hits.size > 0)}<p class="note">{g.note}</p>{/if}
          {#each g.controls as c}
            {@const off = !isActive(params, c.path)}
            {@const hit = hits.get(c.path)}
            {#if shown(hits, c.path)}
            <div class="ctl" class:hit>
            {#if c.kind === "range"}
              <label class="row" class:off title={help[c.path]}>
                <span class="name">{@render label(c.label, hit)}</span>
                <input
                  type="range"
                  disabled={off}
                  min={c.min}
                  max={c.max}
                  step={c.step}
                  value={getPath(params, c.path)}
                  oninput={(e) => setPath(params, c.path, Number((e.target as HTMLInputElement).value))}
                  ondblclick={() => defaults && setPath(params, c.path, getPath(defaults, c.path))}
                  title={`${help[c.path] ?? ""}\n(ダブルクリックで既定値)`.trim()}
                />
                <span class="val">{fmt(getPath(params, c.path), c.step)}{c.unit ? ` ${c.unit}` : ""}</span>
              </label>
            {:else if c.kind === "bool"}
              <label class="row check" class:off title={help[c.path]}>
                <input
                  type="checkbox"
                  disabled={off}
                  checked={getPath(params, c.path)}
                  onchange={(e) => setPath(params, c.path, (e.target as HTMLInputElement).checked)}
                />
                <span>{@render label(c.label, hit)}</span>
              </label>
            {:else if c.kind === "text"}
              <label class="row" class:off title={help[c.path]}>
                <span class="name">{@render label(c.label, hit)}</span>
                <input
                  class="wide"
                  type="text"
                  disabled={off}
                  placeholder={c.placeholder}
                  value={getPath(params, c.path)}
                  onchange={(e) => setPath(params, c.path, (e.target as HTMLInputElement).value)}
                />
              </label>
            {:else if c.kind === "font"}
              <div class="row" class:off title={help[c.path]}>
                <span class="name">{@render label(c.label, hit)}</span>
                <input
                  type="text"
                  disabled={off}
                  placeholder="内蔵 VCR-JP ドットフォント"
                  value={getPath(params, c.path)}
                  onchange={(e) => setPath(params, c.path, (e.target as HTMLInputElement).value.trim())}
                />
                <span class="btns">
                  <button class="small" disabled={off} onclick={() => pickFont(c.path)} title="フォントファイルを選ぶ">…</button>
                  <button class="small" disabled={off} onclick={() => setPath(params, c.path, "")} title="内蔵 VCR-JP フォントに戻す">内蔵</button>
                </span>
              </div>
              {#if fontStatus && !off}
                <p class="note" class:err={fontStatus.error}>{fontStatus.text}</p>
              {/if}
            {:else if c.kind === "markup"}
              <label class="stack" class:off title={help[c.path]}>
                <span class="name">{@render label(c.label, hit)}</span>
                <textarea
                  rows="6"
                  disabled={off}
                  spellcheck="false"
                  value={getPath(params, c.path)}
                  oninput={(e) => setPath(params, c.path, (e.target as HTMLTextAreaElement).value)}
                  ondblclick={(e) => {
                    if (e.altKey && defaults) setPath(params, c.path, getPath(defaults, c.path));
                  }}
                  title={`${help[c.path] ?? ""}\n(Alt+ダブルクリックで既定の内容に戻す)`.trim()}
                ></textarea>
              </label>
            {:else}
              <label class="row" class:off title={help[c.path]}>
                <span class="name">{@render label(c.label, hit)}</span>
                <select
                  disabled={off}
                  value={getPath(params, c.path)}
                  onchange={(e) => setControl(c.path, (e.target as HTMLSelectElement).value)}
                >
                  {#each c.options as o}<option value={o.value}>{o.label}</option>{/each}
                </select>
              </label>
            {/if}
            </div>
            {/if}
          {/each}
        </details>
      {/each}

      {@const crtMatch = crtHits.rows.size + crtHits.params.size}
      <details
        open={isOpen("crt", crtMatch > 0)}
        ontoggle={(e) => onToggle("crt", e)}
        class:nomatch={searching && crtMatch === 0}
      >
        <summary>{CRT_TITLE}{#if searching && crtMatch > 0}<span class="hits">{crtMatch}</span>{/if}</summary>
        <p class="note" class:hidden={searching && crtMatch > 0}>
          RetroArch の slang シェーダープリセット (.slangp) を最終出力に掛ける (例: RetroArch の
          shaders/shaders_slang/crt/crt-royale.slangp)。プレビューは画面の画素数、録画・書き出しは下の高さで描く。
        </p>
        <label class="row check" class:hit={crtHits.rows.has("enabled")}>
          <input type="checkbox" bind:checked={params.crt.enabled} />
          <span>{@render label("シェーダーを掛ける", crtHits.rows.get("enabled"))}</span>
        </label>
        <div class="row" class:hit={crtHits.rows.has("preset")}>
          <span class="name">{@render label("プリセット", crtHits.rows.get("preset"))}</span>
          <input
            type="text"
            placeholder="未選択"
            value={params.crt.preset}
            onchange={(e) => params && (params.crt.preset = (e.target as HTMLInputElement).value.trim())}
            title={params.crt.preset}
          />
          <span class="btns">
            <button class="small" onclick={pickCrtPreset} title=".slangp を選ぶ">…</button>
            <button class="small" onclick={() => invoke("reload_crt")} title="シェーダーを読み直す">⟳</button>
          </span>
        </div>
        <label class="row" class:off={!params.crt.enabled} class:hit={crtHits.rows.has("height")}>
          <span class="name">{@render label("録画・書き出しの高さ", crtHits.rows.get("height"))}</span>
          <select bind:value={params.crt.height} disabled={!params.crt.enabled}>
            {#each CRT_HEIGHTS as h}<option value={h}>{Math.round((h * 4) / 3)}×{h}</option>{/each}
          </select>
        </label>
        {#if params.crt.enabled && params.crt.preset && crtInfo && crtInfo.preset === params.crt.preset}
          {#if crtInfo.loading}
            <p class="note">シェーダーを読み込み中…</p>
          {:else if crtInfo.error}
            <p class="note err">{crtInfo.error}</p>
          {:else}
            <p class="note">GPU: {crtInfo.gpu ?? "?"} · パラメータ {crtInfo.params.filter((p) => p.min !== p.max).length} 個</p>
          {/if}
        {/if}
        {#if crtVisible.length > 0 || crtFilter}
          <details open={isOpen("crt-params", crtHits.params.size > 0)} ontoggle={(e) => onToggle("crt-params", e)}>
            <summary>シェーダーパラメータ{#if searching && crtHits.params.size > 0}<span class="hits">{crtHits.params.size}</span>{/if}</summary>
            <div class="crt-tools">
              <input type="search" placeholder="絞り込み" bind:value={crtFilter} />
              <button
                class="small"
                onclick={() => params && (params.crt.params = {})}
                disabled={Object.keys(params.crt.params).length === 0}
                title="すべてプリセットの値に戻す">既定に戻す</button
              >
            </div>
            {#each crtVisible as p (p.id)}
              {#if p.min === p.max}
                <p class="crt-head">{p.description}</p>
              {:else}
                <label class="row" class:off={!params.crt.enabled} class:hit={crtHits.params.has(p.id)} title={`${p.id}\n(ダブルクリックでプリセットの値)`}>
                  <span class="name" class:changed={p.id in params.crt.params}>{@render label(p.description, crtHits.params.get(p.id))}</span>
                  <input
                    type="range"
                    disabled={!params.crt.enabled}
                    min={p.min}
                    max={p.max}
                    step={p.step}
                    value={crtValue(p)}
                    oninput={(e) => setCrtValue(p, Number((e.target as HTMLInputElement).value))}
                    ondblclick={() => setCrtValue(p, null)}
                  />
                  <span class="val">{fmt(crtValue(p), p.step)}</span>
                </label>
              {/if}
            {/each}
          </details>
        {/if}
      </details>
    {/if}
  </aside>
</main>

<style>
  :global(:root) {
    --bg: #15161a;
    --panel: #1d1f25;
    --line: #2c2f38;
    --text: #d9dbe1;
    --muted: #8a8f9c;
    --accent: #5aa9ff;
    --danger: #ff5a5a;
    color-scheme: dark;
  }
  :global(html, body) {
    margin: 0;
    height: 100%;
    background: var(--bg);
    color: var(--text);
    font-family: "Segoe UI", "Yu Gothic UI", "Hiragino Sans", sans-serif;
    font-size: 13px;
  }
  main {
    display: grid;
    grid-template-columns: 1fr 360px;
    /* 行を画面の高さに固定する (auto だと中身に合わせて伸び、パネルのスクロールが効かない) */
    grid-template-rows: minmax(0, 1fr);
    height: 100vh;
    overflow: hidden;
  }
  .stage {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .toolbar,
  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px;
    border-bottom: 1px solid var(--line);
  }
  .status {
    border-top: 1px solid var(--line);
    border-bottom: none;
    color: var(--muted);
    gap: 14px;
    font-variant-numeric: tabular-nums;
  }
  .perf-display {
    flex: 1 1 500px;
    min-width: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    align-items: center;
    gap: 12px;
  }
  .perf-summary {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .perf-chart {
    min-width: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 8px;
  }
  .perf-total {
    white-space: nowrap;
  }
  .perf-track {
    position: relative;
    display: flex;
    height: 8px;
    min-width: 0;
    overflow: hidden;
    border-radius: 4px;
    background: #30333c;
    box-shadow: inset 0 0 0 1px #414550;
  }
  .perf-track.over-budget {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .perf-segment {
    height: 100%;
    flex-shrink: 0;
  }
  .perf-camera { background: #6da7ff; }
  .perf-encode { background: #b78cff; }
  .perf-tape { background: #e6a34a; }
  .perf-rf { background: #f0d35c; }
  .perf-decode { background: #5ec8d5; }
  .perf-capture { background: #75c98b; }
  .perf-crt { background: #ef7885; }
  .perf-total.over-budget-text { color: var(--danger); }
  .spacer {
    flex: 1;
  }
  .screen {
    position: relative;
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #000;
    min-height: 0;
    padding: 8px;
  }
  canvas {
    /* 754x480 の非正方画素を 4:3 で表示 */
    aspect-ratio: 4 / 3;
    max-width: 100%;
    max-height: 100%;
    height: 100%;
    image-rendering: auto;
  }
  .panel {
    background: var(--panel);
    border-left: 1px solid var(--line);
    min-height: 0;
    overflow-y: auto;
    padding: 8px 10px 24px;
  }
  .presets,
  .seed {
    display: flex;
    gap: 6px;
    margin-bottom: 8px;
    align-items: center;
  }
  .presets select {
    flex: 1;
  }
  .seed input {
    flex: 1;
    min-width: 0;
  }
  details {
    border-top: 1px solid var(--line);
    padding: 6px 0;
  }
  .search {
    position: sticky;
    top: -8px;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 6px;
    margin: -8px -10px 8px;
    padding: 8px 10px 6px;
    background: var(--panel);
  }
  .search input {
    flex: 1;
    min-width: 0;
  }
  .search .count {
    color: var(--muted);
    font-size: 11.5px;
    white-space: nowrap;
  }
  .search .count.none {
    color: #e07a7a;
  }
  details.nomatch > summary {
    color: #5a5e69;
    font-weight: 400;
  }
  summary .hits {
    margin-left: 6px;
    padding: 0 6px;
    border-radius: 8px;
    background: #2a3b52;
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
  }
  mark {
    background: none;
    color: var(--accent);
    font-weight: 600;
  }
  .hidden {
    display: none;
  }
  summary {
    cursor: pointer;
    font-weight: 600;
    padding: 4px 0;
  }
  .row {
    display: grid;
    grid-template-columns: 118px 1fr 78px;
    align-items: center;
    gap: 6px;
    padding: 2px 0;
  }
  .row.check {
    grid-template-columns: auto 1fr;
  }
  .btns {
    display: flex;
    gap: 4px;
  }
  .row select,
  .row .wide {
    grid-column: 2 / 4;
  }
  .stack {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px 0;
  }
  textarea {
    background: #101115;
    color: var(--text);
    border: 1px solid #3a3e4a;
    border-radius: 4px;
    padding: 6px;
    font: 12px/1.45 "Cascadia Mono", Consolas, "MS Gothic", monospace;
    resize: vertical;
    white-space: pre;
  }
  .note {
    margin: 2px 0 6px;
    color: var(--muted);
    font-size: 11.5px;
    line-height: 1.5;
  }
  .name {
    color: var(--muted);
  }
  .val {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  input[type="range"] {
    width: 100%;
  }
  /* フォームコントロールのテーマ。コンポーネントのスタイルはスコープされるので、
     Scopes・Oscilloscope・SyncView・Menu の中のボタンにも効くよう :global にする
     (:where で詳細度を 0 にして、各コンポーネントの個別の指定が勝つようにする) */
  :global(:where(button, select, input[type="number"], input[type="text"], input[type="search"])) {
    background: #2a2d36;
    color: var(--text);
    border: 1px solid #3a3e4a;
    border-radius: 4px;
    padding: 4px 8px;
    font: inherit;
  }
  :global(:where(button:hover:not(:disabled))) {
    border-color: var(--accent);
  }
  :global(:where(button, select, input, textarea):disabled) {
    opacity: 0.5;
    cursor: default;
  }
  :global(:where(input[type="range"], input[type="checkbox"], input[type="radio"])) {
    accent-color: var(--accent);
  }
  /* 今の設定では効かないコントロール */
  .off .name,
  .off > span,
  .off .val {
    color: #5a5e69;
  }
  button.rec,
  .rec-label {
    color: var(--danger);
  }
  button.small {
    padding: 1px 6px;
  }
  .err {
    color: var(--danger);
  }
  .toolbar select {
    max-width: 220px;
  }
  .toolbar label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--muted);
    white-space: nowrap;
  }
  .toolbar .crop {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px 10px;
  }
  .toolbar .crop label {
    display: flex;
    align-items: center;
    gap: 4px;
    justify-content: space-between;
  }
  .toolbar input.crop-num {
    width: 64px;
    padding: 3px 5px;
  }
  .toolbar .item .title {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row input[type="text"] {
    min-width: 0;
  }
  .crt-tools {
    display: flex;
    gap: 6px;
    margin: 4px 0 6px;
  }
  .crt-tools input {
    flex: 1;
    min-width: 0;
    background: #2a2d36;
    color: var(--text);
    border: 1px solid #3a3e4a;
    border-radius: 4px;
    padding: 3px 8px;
    font: inherit;
  }
  .crt-head {
    margin: 8px 0 2px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text);
  }
  .name.changed {
    color: var(--accent);
  }
  .panels {
    border-top: 1px solid var(--line);
    background: #0c0d10;
  }
  .panels :global(.scopes),
  .panels :global(.osc) {
    border-top: none;
  }
  .tabs {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    padding: 4px 8px 0;
    border-bottom: 1px solid var(--line);
  }
  .tabs button {
    padding: 3px 12px;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 4px 4px 0 0;
    background: none;
    color: var(--muted);
    margin-bottom: -1px;
  }
  .tabs button:hover {
    color: var(--text);
  }
  .tabs button.on {
    color: var(--text);
    background: #0c0d10;
    border-color: var(--line);
    border-bottom: 1px solid #0c0d10;
    font-weight: 600;
  }
  .tabs .tab-close {
    margin-left: auto;
  }
  /* イマーシブモード: 映像だけを全画面に出す */
  main.immersive {
    grid-template-columns: 1fr;
    background: #000;
  }
  main.immersive .toolbar,
  main.immersive .status,
  main.immersive .panel,
  main.immersive .panels {
    display: none;
  }
  main.immersive .screen {
    padding: 0;
  }
</style>
