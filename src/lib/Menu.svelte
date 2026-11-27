<script lang="ts">
  import type { Snippet } from "svelte";

  // ツールバーのポップアップメニュー。外側のクリックか Esc で閉じる
  let {
    label,
    title = "",
    align = "left",
    onopen,
    children,
  }: {
    label: string;
    title?: string;
    align?: "left" | "right";
    onopen?: () => void;
    children: Snippet<[() => void]>;
  } = $props();

  let open = $state(false);
  let root: HTMLDivElement;

  function close() {
    open = false;
  }

  function toggle() {
    open = !open;
    if (open) onopen?.();
  }

  function onDocPointer(e: PointerEvent) {
    if (open && !root.contains(e.target as Node)) close();
  }

  function onKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") close();
  }
</script>

<svelte:document onpointerdown={onDocPointer} onkeydown={onKey} />

<div class="menu" bind:this={root}>
  <button class="trigger" class:on={open} {title} aria-haspopup="menu" aria-expanded={open} onclick={toggle}>
    {label} <span class="caret">▾</span>
  </button>
  {#if open}
    <div class="pop" class:right={align === "right"} role="menu">
      {@render children(close)}
    </div>
  {/if}
</div>

<style>
  .menu {
    position: relative;
    display: inline-block;
  }
  .trigger.on {
    border-color: var(--accent);
  }
  .caret {
    color: var(--muted);
    font-size: 10px;
  }
  .pop {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 50;
    min-width: 220px;
    max-width: 420px;
    padding: 4px;
    background: var(--panel);
    border: 1px solid #3a3e4a;
    border-radius: 6px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
    display: flex;
    flex-direction: column;
    gap: 2px;
    color: var(--text);
    color-scheme: dark;
    font: inherit;
  }
  .pop.right {
    left: auto;
    right: 0;
  }
  /* メニュー項目 (中身側で class="item" を付ける) */
  .pop :global(.item) {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    width: 100%;
    box-sizing: border-box;
    min-height: 26px;
    font: inherit;
    line-height: 1.4;
    text-align: left;
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    padding: 5px 8px;
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pop :global(.item:hover:not(:disabled)) {
    background: #2a2d36;
    border-color: transparent;
  }
  .pop :global(.item.current) {
    color: var(--accent);
  }
  .pop :global(.item .sub) {
    margin-left: auto;
    color: var(--muted);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }
  .pop :global(.sep) {
    height: 1px;
    background: var(--line);
    margin: 4px 2px;
  }
  .pop :global(.head) {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px 2px;
    color: var(--muted);
    font-size: 11.5px;
  }
  .pop :global(.head button) {
    margin-left: auto;
    padding: 0 6px;
  }
  .pop :global(.field) {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    color: var(--muted);
    white-space: nowrap;
  }
  .pop :global(.field select),
  .pop :global(.field input[type="text"]) {
    flex: 1;
    min-width: 0;
  }
  .pop :global(.empty) {
    padding: 5px 8px;
    color: var(--muted);
  }
  .pop :global(.list) {
    max-height: 320px;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-width: thin;
    scrollbar-color: #4a4e5a transparent;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
</style>
