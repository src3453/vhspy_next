<script lang="ts">
  // イマーシブモード: ブラウザの UI を隠し、デッキのメニュー表示 (OSD) で操作する。
  // メニューは内蔵の VCR フォントで描き、エンジン側でデッキの再生出力 (コンポジット / S 映像) に
  // 重畳するので、テープ・RF・受像機・CRT シェーダーの見た目を一緒に受ける。
  //
  // 操作 (リモコン風):
  //   メニューなし: Enter / M = メニュー, ↑↓ = トラッキング, Space = 状態表示, Esc / F11 = 終了
  //   メニュー中:   ↑↓ = 選択, ←→ = 値の変更 (Shift で 10 倍), Enter = 決定, Esc / Backspace = 戻る
  //   マウス:       クリック = メニューの開閉, ホイール = 選択
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { groups, getPath, isActive, presets, type Control, type Params } from "#lib/params.ts";
  import {
    ARROW, controlLabels, escapeMarkup, groupLabels, optionLabels, osdSafe, osdUnit, padEnd, padStart, presetLabels, toOsd,
  } from "#lib/osdText.ts";

  type Actions = {
    openPattern: () => void;
    openCamera: () => void;
    openFile: () => void;
    refreshCameras: () => Promise<void>;
    toggleRecording: () => void;
    applyPreset: (i: number) => void;
    /** 値を変える (方式・フォーマットの切り替えは関連する値も合わせる) */
    setControl: (path: string, v: unknown) => void;
    exit: () => void;
  };

  let {
    params,
    defaults,
    source,
    recording,
    message,
    cameras,
    camera = $bindable(),
    actions,
  }: {
    params: Params;
    defaults: Params | null;
    source: string;
    recording: boolean;
    message: string;
    cameras: string[];
    camera: string;
    actions: Actions;
  } = $props();

  /** メニュー項目 */
  type Item = {
    label: string;
    value?: () => string;
    /** 範囲のある値の位置 (0..1)。選択中だけ下にバーを出す */
    bar?: () => number;
    enter?: () => void;
    adjust?: (dir: number, big: boolean) => void;
    dim?: () => boolean;
    /** 開くページ */
    sub?: string;
  };
  type Page = { title: string; items: Item[] };

  // 表示幅 (文字) と行
  const COLS = 30;
  const VALUE_COLS = 12;
  const PITCH = 32; // 行の間隔 [ライン] (背景の帯 = 文字 24 + 上下 4 ずつ)

  let menuOpen = $state(false);
  let stack = $state<{ page: string; cursor: number }[]>([{ page: "root", cursor: 0 }]);
  let toast = $state<{ label: string; value: string; bar: number | null; until: number } | null>(null);
  let statusUntil = $state(0);
  let now = $state(performance.now());
  let presetIndex = $state(0);

  const yes = (v: boolean) => (v ? "ON" : "OFF");

  function fmt(v: number, step: number) {
    const d = step >= 1 ? 0 : Math.min(4, Math.ceil(-Math.log10(step)));
    return v.toFixed(d);
  }

  function controlItem(c: Control): Item | null {
    const label = controlLabels[c.path] ?? toOsd(c.path.split(".").pop()!.toUpperCase());
    const dim = () => !isActive(params, c.path);
    switch (c.kind) {
      case "range": {
        const get = () => Number(getPath(params, c.path));
        return {
          label,
          dim,
          value: () => fmt(get(), c.step) + osdUnit(c.unit),
          bar: () => (get() - c.min) / (c.max - c.min),
          adjust: (dir, big) => {
            // 細かい刻みのものは 1 回で全体の 1/100 は動かす
            const base = Math.max(c.step, Math.round((c.max - c.min) / 100 / c.step) * c.step);
            const v = Math.min(c.max, Math.max(c.min, get() + dir * base * (big ? 10 : 1)));
            actions.setControl(c.path, Number(v.toFixed(6)));
          },
          enter: () => defaults && actions.setControl(c.path, getPath(defaults, c.path)),
        };
      }
      case "bool": {
        const toggle = () => actions.setControl(c.path, !getPath(params, c.path));
        return { label, dim, value: () => yes(!!getPath(params, c.path)), adjust: toggle, enter: toggle };
      }
      case "select": {
        const name = (v: string) => {
          const o = c.options.find((o) => o.value === v);
          const own = optionLabels[c.path]?.[v];
          if (own) return own;
          if (o && osdSafe(o.label)) return toOsd(o.label);
          return toOsd(v.toUpperCase());
        };
        const cycle = (dir: number) => {
          const i = c.options.findIndex((o) => o.value === getPath(params, c.path));
          const n = c.options.length;
          actions.setControl(c.path, c.options[(((i + dir) % n) + n) % n].value);
        };
        return { label, dim, value: () => name(getPath(params, c.path)), adjust: (d) => cycle(d), enter: () => cycle(1) };
      }
      default:
        // 文字の入力が要るもの (マークアップ・フォント・日時) は OSD では扱わない
        return null;
    }
  }

  function presetName(i: number) {
    const n = presets[i]?.name ?? "";
    return presetLabels[n] ?? (osdSafe(n) ? toOsd(n) : `PRESET ${i + 1}`);
  }

  function page(id: string): Page {
    if (id === "root") {
      return {
        title: "MENU",
        items: [
          { label: "ﾆｭｳﾘｮｸ", sub: "source" },
          {
            label: "ﾌﾟﾘｾｯﾄ",
            value: () => presetName(presetIndex),
            adjust: (d) => (presetIndex = (presetIndex + d + presets.length) % presets.length),
            enter: () => {
              actions.applyPreset(presetIndex);
              flash("ﾌﾟﾘｾｯﾄ", presetName(presetIndex));
            },
          },
          { label: "ｾｯﾃｲ", sub: "settings" },
          { label: "ﾛｸｶﾞ", value: () => (recording ? "REC" : "STOP"), enter: actions.toggleRecording },
          {
            label: "CRT ｼｪｰﾀﾞｰ",
            value: () => yes(params.crt.enabled),
            dim: () => !params.crt.preset,
            adjust: () => actions.setControl("crt.enabled", !params.crt.enabled),
            enter: () => actions.setControl("crt.enabled", !params.crt.enabled),
          },
          {
            label: "ｼｰﾄﾞ",
            value: () => String(params.seed),
            adjust: (d, big) => actions.setControl("seed", Math.max(0, params.seed + d * (big ? 100 : 1))),
            enter: () => actions.setControl("seed", Math.floor(Math.random() * 2 ** 31)),
          },
          { label: "ｲﾏｰｼﾌﾞ ｼｭｳﾘｮｳ", enter: actions.exit },
        ],
      };
    }
    if (id === "source") {
      return {
        title: "ﾆｭｳﾘｮｸ",
        items: [
          { label: "ﾃｽﾄﾊﾟﾀｰﾝ", enter: actions.openPattern },
          {
            label: "ｶﾒﾗ",
            value: () => (camera ? toOsd(camera) : "---"),
            dim: () => cameras.length === 0,
            adjust: (d) => {
              if (cameras.length === 0) return;
              const i = cameras.indexOf(camera);
              camera = cameras[(i + d + cameras.length) % cameras.length];
            },
            enter: () => camera && actions.openCamera(),
          },
          { label: "ｶﾒﾗ ｲﾁﾗﾝ ｺｳｼﾝ", enter: () => void actions.refreshCameras() },
          { label: "ﾌｧｲﾙ...", enter: actions.openFile },
        ],
      };
    }
    if (id === "settings") {
      return {
        title: "ｾｯﾃｲ",
        items: groups.map((g, i) => ({ label: groupLabels[g.title] ?? toOsd(g.title), sub: `g${i}` })),
      };
    }
    const g = groups[Number(id.slice(1))];
    return {
      title: groupLabels[g.title] ?? toOsd(g.title),
      items: g.controls.map(controlItem).filter((x): x is Item => x !== null),
    };
  }

  let current = $derived(page(stack[stack.length - 1].page));

  /** メニューを閉じているときの一時表示 */
  function flash(label: string, value: string, bar: number | null = null) {
    toast = { label, value, bar, until: performance.now() + 2500 };
  }

  function move(d: number) {
    const top = stack[stack.length - 1];
    const n = current.items.length;
    top.cursor = (top.cursor + d + n) % n;
  }

  function back() {
    if (stack.length > 1) stack.pop();
    else menuOpen = false;
  }

  function enter() {
    const it = current.items[stack[stack.length - 1].cursor];
    if (!it) return;
    if (it.sub) stack.push({ page: it.sub, cursor: 0 });
    else it.enter?.();
  }

  // トラッキング (メニューを閉じているときの ↑↓。デッキのトラッキングボタン風)
  function tracking(dir: number) {
    const v = Math.min(1, Math.max(0, params.transport.tracking_error + dir * 0.02));
    actions.setControl("transport.tracking_error", Number(v.toFixed(3)));
    flash("TRACKING", fmt(v, 0.01), v);
  }

  function onKey(e: KeyboardEvent) {
    const k = e.key;
    if (k === "F11") {
      e.preventDefault();
      actions.exit();
      return;
    }
    if (!menuOpen) {
      if (k === "Enter" || k === "m" || k === "M") {
        menuOpen = true;
        stack = [{ page: "root", cursor: 0 }];
      } else if (k === "Escape") actions.exit();
      else if (k === "ArrowUp") tracking(1);
      else if (k === "ArrowDown") tracking(-1);
      else if (k === " ") statusUntil = performance.now() + 4000;
      else return;
      e.preventDefault();
      return;
    }
    const it = current.items[stack[stack.length - 1].cursor];
    if (k === "ArrowUp") move(-1);
    else if (k === "ArrowDown") move(1);
    else if (k === "ArrowLeft" || k === "ArrowRight") {
      const d = k === "ArrowLeft" ? -1 : 1;
      if (it?.adjust) it.adjust(d, e.shiftKey);
      else if (d > 0 && it?.sub) enter();
      else if (d < 0) back();
    } else if (k === "Enter") enter();
    else if (k === "Escape" || k === "Backspace") back();
    else if (k === "m" || k === "M") menuOpen = false;
    else return;
    e.preventDefault();
  }

  function onClick(e: MouseEvent) {
    if (e.button !== 0) return;
    menuOpen = !menuOpen;
    if (menuOpen) stack = [{ page: "root", cursor: 0 }];
  }

  function onWheel(e: WheelEvent) {
    if (menuOpen) move(e.deltaY > 0 ? 1 : -1);
  }

  // ---- 表示 (OSD マークアップ) ----

  function bar(v: number, cells: number) {
    const halves = Math.round(Math.min(1, Math.max(0, v)) * cells * 2);
    return "█".repeat(halves >> 1) + (halves & 1 ? "▌" : "") + "░".repeat(cells - (halves >> 1) - (halves & 1));
  }

  let markup = $derived.by(() => {
    const [W, H] = params.standard === "pal" ? [922, 576] : [754, 480];
    const x0 = Math.round(W * 0.07);
    const y0 = Math.round(H * 0.07);
    const rows = Math.floor((H - 2 * y0) / PITCH);
    const out: string[] = [];
    const line = (row: number, text: string, color = "white") =>
      out.push(`<pos=${x0},${y0 + row * PITCH}><bg=black><color=${color}>${escapeMarkup(padEnd(text, COLS))}</color></bg>`);

    if (menuOpen) {
      const { cursor } = stack[stack.length - 1];
      const items = current.items;
      const visible = rows - 3; // 見出し・バー・操作説明
      const top = Math.min(Math.max(0, cursor - Math.floor(visible / 2)), Math.max(0, items.length - visible));
      const more = (up: boolean) => (up ? top > 0 : top + visible < items.length);
      line(0, padEnd(" " + current.title, COLS - 2) + (more(true) ? ARROW.up : " ") + " ", "#40ff40");
      for (let r = 0; r < visible; r++) {
        const it = items[top + r];
        if (!it) {
          line(1 + r, "");
          continue;
        }
        const sel = top + r === cursor;
        const val = it.sub ? ARROW.right : (it.value?.() ?? "");
        const text = (sel ? ARROW.right : " ") + padEnd(it.label, COLS - VALUE_COLS - 2) + " " + padStart(val, VALUE_COLS);
        line(1 + r, text, sel ? "yellow" : it.dim?.() ? "#808080" : "white");
      }
      const it = items[cursor];
      const b = it?.bar?.();
      line(rows - 2, b != null ? " -" + bar(b, 20) + "+" : padStart(more(false) ? ARROW.down + " " : "", COLS), "white");
      line(rows - 1, ` ${ARROW.up}${ARROW.down}:ｾﾝﾀｸ ${ARROW.left}${ARROW.right}:ﾍﾝｺｳ ESC:ﾓﾄﾞﾙ`, "cyan");
    } else {
      if (toast && toast.until > now) {
        line(rows - 2, " " + padEnd(toast.label, COLS - VALUE_COLS - 2) + padStart(toast.value, VALUE_COLS) + " ");
        if (toast.bar != null) line(rows - 1, " -" + bar(toast.bar, 20) + "+");
      }
      if (statusUntil > now) {
        const src = source.startsWith("テストパターン") ? "ﾃｽﾄﾊﾟﾀｰﾝ" : source.startsWith("カメラ") ? "CAMERA" : source.startsWith("画像") ? "STILL" : "PLAY " + ARROW.right;
        out.push(`<pos=${x0},${y0}><size=3><edge=black>${escapeMarkup(src)}</edge></size>`);
        out.push(`<pos=${W - x0},${y0 + rows * PITCH - 40}><align=right><size=3><edge=black>{counter}</edge></size>`);
      }
      if (recording) out.push(`<pos=${W - x0},${y0}><align=right><size=3><edge=black><color=red><blink>REC</blink></color></edge></size>`);
    }
    return out.length ? out.join("\n") : null;
  });

  // エンジンへ送る (変わったときだけ)
  let sent: string | null | undefined;
  $effect(() => {
    const m = markup;
    if (m !== sent) {
      sent = m;
      invoke("set_menu_osd", { markup: m });
    }
  });

  // 入力が変わったら状態を表示する
  let lastSource = "";
  $effect(() => {
    if (source !== lastSource) {
      lastSource = source;
      statusUntil = performance.now() + 4000;
    }
  });

  // エラーは短く知らせる (日本語の文はフォントで出せない)
  $effect(() => {
    if (message) flash("ERROR", "");
  });

  let timer: ReturnType<typeof setInterval>;
  onMount(() => {
    statusUntil = performance.now() + 4000;
    timer = setInterval(() => (now = performance.now()), 200);
  });
  onDestroy(() => {
    clearInterval(timer);
    invoke("set_menu_osd", { markup: null });
  });
</script>

<svelte:window onkeydown={onKey} />

<!-- 画面全体でクリック・ホイールを受ける (カーソルは隠す) -->
<div class="catcher" role="presentation" onmousedown={onClick} onwheel={onWheel} oncontextmenu={(e) => e.preventDefault()}></div>

<style>
  .catcher {
    position: fixed;
    inset: 0;
    cursor: none;
    z-index: 10;
  }
</style>
