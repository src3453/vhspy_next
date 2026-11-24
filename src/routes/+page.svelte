<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { groups, help, getPath, setPath, isActive, presets, applyDiff, retarget, type Params } from "#lib/params.ts";
  import Scopes from "#lib/Scopes.svelte";

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

  let params = $state<Params | null>(null);
  let defaults: Params | null = null;
  let stats = $state<Stats | null>(null);
  let cameras = $state<string[]>([]);
  let camera = $state("");
  let message = $state("");
  let exporting = $state<ExportProgress | null>(null);
  let canvas: HTMLCanvasElement;
  let scopes: Scopes | undefined = $state();
  let crtInfo = $state<CrtInfo | null>(null);
  let crtVersion = -1;
  let crtFilter = $state("");
  const CRT_HEIGHTS = [720, 960, 1080, 1440, 2160];

  const VIDEO_EXTS = ["mp4", "mkv", "mov", "avi", "webm", "m4v", "mpg", "ts"];
  const IMAGE_EXTS = ["png", "jpg", "jpeg", "bmp", "webp"];

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
    run(() => invoke("open_source", { req: { kind: "camera", device: camera } }));
  }

  function openPattern() {
    run(() => invoke("open_source", { req: { kind: "pattern" } }));
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
      filters: [{ name: "MP4", extensions: ["mp4"] }],
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
    if (!q) return ps;
    return ps.filter((p) => p.min !== p.max && (p.id.toLowerCase().includes(q) || p.description.toLowerCase().includes(q)));
  });

  function applyPreset(i: number) {
    // 今のテレビ方式とテープのフォーマットは引き継ぐ (プリセットが指定していればそちら)
    if (!defaults) return;
    // FM モデルで使っていたときだけフォーマットの値をすべて引き継ぐ (簡易モデルの既定の見た目は変えない)
    const fm = params?.tape.luma_model === "fm";
    const base = retarget(defaults, params?.standard ?? "ntsc", params?.tape.format ?? "vhs", fm);
    const crt = params ? $state.snapshot(params.crt) : null;
    params = applyDiff(base, presets[i].diff);
    // ブラウン管の設定は表示環境なので VHS のプリセットでは変えない
    if (crt && params) params.crt = crt;
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

  function newSeed() {
    if (params) params.seed = Math.floor(Math.random() * 2 ** 31);
  }

  onMount(() => {
    let alive = true;
    let lastId = 0;
    const ctx = canvas.getContext("2d")!;

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

<main>
  <section class="stage">
    <div class="toolbar">
      <button onclick={openPattern}>テストパターン</button>
      <select bind:value={camera} title="カメラ">
        {#each cameras as c}<option value={c}>{c}</option>{/each}
      </select>
      <button onclick={() => run(refreshCameras)} title="カメラ一覧を更新">⟳</button>
      <button onclick={openCamera} disabled={!camera}>カメラを開く</button>
      <button onclick={openFile}>ファイルを開く…</button>
      <span class="spacer"></span>
      <button class:rec={!!stats?.recording} onclick={toggleRecording}>
        {stats?.recording ? "■ 録画停止" : "● 録画"}
      </button>
      <button onclick={startExport} disabled={!!exporting}>動画を書き出し…</button>
    </div>

    <div class="screen">
      <canvas bind:this={canvas} width="754" height="480"></canvas>
    </div>

    <Scopes bind:this={scopes} standard={params?.standard ?? "ntsc"} />

    <div class="status">
      <span>{stats?.source ?? "…"}</span>
      {#if stats}
        <span>{stats.fields_per_sec.toFixed(1)} fields/s</span>
        <span>
          処理 {(stats.camera_ms + stats.encode_ms + stats.tape_ms + stats.rf_ms + stats.decode_ms + stats.capture_ms).toFixed(2)} ms
          ({stats.camera_ms > 0.01 ? `cam ${stats.camera_ms.toFixed(2)} / ` : ""}enc {stats.encode_ms.toFixed(2)} / tape {stats.tape_ms.toFixed(2)}{stats.rf_ms > 0.01
            ? ` / rf ${stats.rf_ms.toFixed(2)}`
            : ""} / dec {stats.decode_ms.toFixed(2)}{stats.capture_ms > 0.01 ? ` / cap ${stats.capture_ms.toFixed(2)}` : ""}{stats.crt_ms > 0.01
            ? ` / crt ${stats.crt_ms.toFixed(2)}`
            : ""})
        </span>
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
      <div class="seed">
        <label for="seed">シード</label>
        <input id="seed" type="number" bind:value={params.seed} />
        <button onclick={newSeed}>🎲</button>
      </div>
      {#each groups as g, gi}
        <details open={gi < 2}>
          <summary>{g.title}</summary>
          {#if g.note}<p class="note">{g.note}</p>{/if}
          {#each g.controls as c}
            {@const off = !isActive(params, c.path)}
            {#if c.kind === "range"}
              <label class="row" class:off title={help[c.path]}>
                <span class="name">{c.label}</span>
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
                <span>{c.label}</span>
              </label>
            {:else if c.kind === "text"}
              <label class="row" class:off title={help[c.path]}>
                <span class="name">{c.label}</span>
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
                <span class="name">{c.label}</span>
                <input
                  type="text"
                  disabled={off}
                  placeholder="内蔵 5×7 ドット"
                  value={getPath(params, c.path)}
                  onchange={(e) => setPath(params, c.path, (e.target as HTMLInputElement).value.trim())}
                />
                <span class="btns">
                  <button class="small" disabled={off} onclick={() => pickFont(c.path)} title="フォントファイルを選ぶ">…</button>
                  <button class="small" disabled={off} onclick={() => setPath(params, c.path, "")} title="内蔵フォントに戻す">内蔵</button>
                </span>
              </div>
              {#if fontStatus && !off}
                <p class="note" class:err={fontStatus.error}>{fontStatus.text}</p>
              {/if}
            {:else if c.kind === "markup"}
              <label class="stack" class:off title={help[c.path]}>
                <span class="name">{c.label}</span>
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
                <span class="name">{c.label}</span>
                <select
                  disabled={off}
                  value={getPath(params, c.path)}
                  onchange={(e) => {
                    const v = (e.target as HTMLSelectElement).value;
                    // 方式・フォーマットの切り替えは、それに合う値 (帯域・搬送波・電源周波数など) もまとめて合わせる
                    // ($state のプロキシは structuredClone できないので、スナップショットを渡す)
                    if (c.path === "standard" || c.path === "tape.format") {
                      const snap = $state.snapshot(params) as Params;
                      setPath(snap, c.path, v);
                      params = retarget(snap, snap.standard, snap.tape.format, c.path === "tape.format");
                    } else setPath(params, c.path, v);
                  }}
                >
                  {#each c.options as o}<option value={o.value}>{o.label}</option>{/each}
                </select>
              </label>
            {/if}
          {/each}
        </details>
      {/each}

      <details open={params.crt.enabled}>
        <summary>ブラウン管 (CRT シェーダー)</summary>
        <p class="note">
          RetroArch の slang シェーダープリセット (.slangp) を最終出力に掛ける (例: RetroArch の
          shaders/shaders_slang/crt/crt-royale.slangp)。プレビューは画面の画素数、録画・書き出しは下の高さで描く。
        </p>
        <label class="row check">
          <input type="checkbox" bind:checked={params.crt.enabled} />
          <span>シェーダーを掛ける</span>
        </label>
        <div class="row">
          <span class="name">プリセット</span>
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
        <label class="row" class:off={!params.crt.enabled}>
          <span class="name">録画・書き出しの高さ</span>
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
          <details>
            <summary>シェーダーパラメータ</summary>
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
                <label class="row" class:off={!params.crt.enabled} title={`${p.id}\n(ダブルクリックでプリセットの値)`}>
                  <span class="name" class:changed={p.id in params.crt.params}>{p.description}</span>
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
  .spacer {
    flex: 1;
  }
  .screen {
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
    accent-color: var(--accent);
  }
  button,
  select,
  input[type="number"],
  input[type="text"] {
    background: #2a2d36;
    color: var(--text);
    border: 1px solid #3a3e4a;
    border-radius: 4px;
    padding: 4px 8px;
    font: inherit;
  }
  button:hover:not(:disabled) {
    border-color: var(--accent);
  }
  button:disabled,
  select:disabled,
  input:disabled,
  textarea:disabled {
    opacity: 0.5;
    cursor: default;
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
</style>
