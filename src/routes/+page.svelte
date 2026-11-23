<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { groups, getPath, setPath, presets, applyDiff, type Params } from "$lib/params";

  type Stats = {
    source: string;
    fields_per_sec: number;
    encode_ms: number;
    tape_ms: number;
    decode_ms: number;
    recording: string | null;
    error: string | null;
  };
  type ExportProgress = { done: number; total: number | null; finished: boolean; error: string | null };

  let params = $state<Params | null>(null);
  let defaults: Params | null = null;
  let stats = $state<Stats | null>(null);
  let cameras = $state<string[]>([]);
  let camera = $state("");
  let message = $state("");
  let exporting = $state<ExportProgress | null>(null);
  let canvas: HTMLCanvasElement;

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

  function applyPreset(i: number) {
    if (defaults) params = applyDiff(defaults, presets[i].diff);
  }

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
            ctx.putImageData(new ImageData(new Uint8ClampedArray(buf, 16, w * h * 4), w, h), 0, 0);
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
    }, 500);

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

    <div class="status">
      <span>{stats?.source ?? "…"}</span>
      {#if stats}
        <span>{stats.fields_per_sec.toFixed(1)} fields/s</span>
        <span>
          処理 {(stats.encode_ms + stats.tape_ms + stats.decode_ms).toFixed(2)} ms
          (enc {stats.encode_ms.toFixed(2)} / tape {stats.tape_ms.toFixed(2)} / dec {stats.decode_ms.toFixed(2)})
        </span>
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
          {#each g.controls as c}
            {#if c.kind === "range"}
              <label class="row">
                <span class="name">{c.label}</span>
                <input
                  type="range"
                  min={c.min}
                  max={c.max}
                  step={c.step}
                  value={getPath(params, c.path)}
                  oninput={(e) => setPath(params, c.path, Number((e.target as HTMLInputElement).value))}
                  ondblclick={() => defaults && setPath(params, c.path, getPath(defaults, c.path))}
                  title="ダブルクリックで既定値"
                />
                <span class="val">{fmt(getPath(params, c.path), c.step)}{c.unit ? ` ${c.unit}` : ""}</span>
              </label>
            {:else if c.kind === "bool"}
              <label class="row check">
                <input
                  type="checkbox"
                  checked={getPath(params, c.path)}
                  onchange={(e) => setPath(params, c.path, (e.target as HTMLInputElement).checked)}
                />
                <span>{c.label}</span>
              </label>
            {:else}
              <label class="row">
                <span class="name">{c.label}</span>
                <select
                  value={getPath(params, c.path)}
                  onchange={(e) => setPath(params, c.path, (e.target as HTMLSelectElement).value)}
                >
                  {#each c.options as o}<option value={o.value}>{o.label}</option>{/each}
                </select>
              </label>
            {/if}
          {/each}
        </details>
      {/each}
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
    height: 100vh;
  }
  .stage {
    display: flex;
    flex-direction: column;
    min-width: 0;
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
  .row select {
    grid-column: 2 / 4;
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
  input[type="number"] {
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
  button:disabled {
    opacity: 0.5;
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
</style>
