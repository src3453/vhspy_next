<script lang="ts">
  // デコード画面の上に、オシロスコープが見ているラインを重ねて示す。
  // 映像のあるラインはその行を光らせ、帰線区間 (画面外) のラインは上下の端に印を付ける。
  // 親 (.screen) は position: relative にしておくこと。
  import { scopeView, imageRow, imageLines, fieldOf, offscreenAbove, selectedLines } from "#lib/scopeView.svelte.ts";

  let { target, hidden = false }: { target: HTMLCanvasElement | undefined; hidden?: boolean } = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  /** 対象の画像の、親の中での位置 [CSS px] */
  let box = $state({ x: 0, y: 0, w: 0, h: 0 });

  $effect(() => {
    const el = target;
    const parent = el?.parentElement;
    if (!el || !parent) return;
    const measure = () => {
      const r = el.getBoundingClientRect();
      const p = parent.getBoundingClientRect();
      box = { x: r.left - p.left, y: r.top - p.top, w: r.width, h: r.height };
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    ro.observe(parent);
    return () => ro.disconnect();
  });

  const show = $derived(scopeView.active && !hidden && box.w > 0);

  $effect(() => {
    if (!show || !canvas) return;
    const { standard: std, line, count } = scopeView;
    const { w, h } = box;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.max(1, Math.round(w * dpr));
    canvas.height = Math.max(1, Math.round(h * dpr));
    const ctx = canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const H = imageLines(std);
    const rowH = h / H;
    const nums = selectedLines(std, line, count);
    const rows: { row: number; field: number }[] = [];
    let above = 0,
      below = 0;
    const fields = new Set<number>();
    for (const n of nums) {
      fields.add(fieldOf(std, n));
      const row = imageRow(std, n);
      if (row !== null) rows.push({ row, field: row & 1 });
      // 映像の前の帰線区間は上、後ろ (フィールドの終わりの半ライン・等化) は下
      else if (offscreenAbove(std, n)) above++;
      else below++;
    }

    // 行を光らせる (細くなりすぎないよう最低 1.5 px)
    const hl = Math.max(rowH, 1.5);
    ctx.fillStyle = "rgba(255, 210, 80, 0.45)";
    for (const { row } of rows) ctx.fillRect(0, row * rowH + rowH / 2 - hl / 2, w, hl);

    // 左右の端に、続いている行のまとまりごとの括弧
    rows.sort((a, b) => a.field - b.field || a.row - b.row);
    ctx.fillStyle = "#ffd27a";
    for (let i = 0; i < rows.length; ) {
      let j = i;
      while (j + 1 < rows.length && rows[j + 1].field === rows[i].field && rows[j + 1].row === rows[j].row + 2) j++;
      const y0 = rows[i].row * rowH;
      const y1 = (rows[j].row + 1) * rowH;
      const ym = (y0 + y1) / 2;
      const len = Math.max(y1 - y0, 8);
      for (const [x, dir] of [
        [0, 1],
        [w, -1],
      ]) {
        ctx.fillRect(dir > 0 ? x : x - 3, ym - len / 2, 3, len);
        ctx.beginPath();
        ctx.moveTo(x, ym - 6);
        ctx.lineTo(x + dir * 9, ym);
        ctx.lineTo(x, ym + 6);
        ctx.fill();
      }
      i = j + 1;
    }

    // 画面外 (帰線区間) のラインは端に点線
    ctx.strokeStyle = "#ffd27a";
    ctx.lineWidth = 3;
    ctx.setLineDash([6, 4]);
    if (above) {
      ctx.beginPath();
      ctx.moveTo(0, 1.5);
      ctx.lineTo(w, 1.5);
      ctx.stroke();
    }
    if (below) {
      ctx.beginPath();
      ctx.moveTo(0, h - 1.5);
      ctx.lineTo(w, h - 1.5);
      ctx.stroke();
    }
    ctx.setLineDash([]);

    // 説明
    const first = nums[0],
      last = nums[nums.length - 1];
    const fieldText = [...fields].sort().map((f) => (f === 0 ? "第 1" : "第 2")).join("・") + " フィールド";
    let text = `OSC ライン ${first}${nums.length > 1 ? `–${last}` : ""} · ${fieldText}`;
    if (above + below) text += rows.length ? ` · 画面外 ${above + below} ライン` : " · 帰線区間 (画面外)";
    ctx.font = "12px 'Segoe UI', sans-serif";
    ctx.textBaseline = "top";
    const tw = ctx.measureText(text).width;
    const ty = above && !below ? h - 24 : 8;
    ctx.fillStyle = "rgba(0, 0, 0, 0.65)";
    ctx.fillRect(8, ty, tw + 12, 18);
    ctx.fillStyle = "#ffd27a";
    ctx.fillText(text, 14, ty + 3);
  });
</script>

{#if show}
  <canvas
    class="scope-overlay"
    bind:this={canvas}
    style:left="{box.x}px"
    style:top="{box.y}px"
    style:width="{box.w}px"
    style:height="{box.h}px"
  ></canvas>
{/if}

<style>
  .scope-overlay {
    position: absolute;
    pointer-events: none;
  }
</style>
