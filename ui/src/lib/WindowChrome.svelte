<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { t } from "./i18n.svelte";
  import SystemMeter from "./SystemMeter.svelte";
  import InstanceSwitcher from "./InstanceSwitcher.svelte";

  /**
   * The window's own chrome. The OS title bar is off (`decorations: false`),
   * so this strip carries the drag region and the three window buttons,
   * drawn in the app's palette. Double-clicking the strip toggles maximize;
   * Tauri handles that through the drag-region attribute.
   *
   * The instance switcher (this PC or a remote instance) sits at the left.
   * The empty middle is on purpose: a command bar and notifications go
   * there later. The system meter sits by the window buttons.
   */
  const win = getCurrentWindow();
  let maximized = $state(false);

  async function refresh() {
    try {
      maximized = await win.isMaximized();
    } catch {
      maximized = false;
    }
  }

  onMount(() => {
    refresh();
    let unlisten: (() => void) | undefined;
    win.onResized(() => refresh()).then((u) => (unlisten = u));
    return () => unlisten?.();
  });
</script>

<div class="chrome" data-tauri-drag-region>
  <InstanceSwitcher />
  <span class="grow" data-tauri-drag-region></span>
  <SystemMeter />
  <div class="controls">
    <button class="wc" onclick={() => win.minimize()} aria-label={t("win.minimize")} title={t("win.minimize")}>
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" stroke="currentColor" stroke-width="1" /></svg>
    </button>
    <button class="wc" onclick={() => win.toggleMaximize()} aria-label={maximized ? t("win.restore") : t("win.maximize")} title={maximized ? t("win.restore") : t("win.maximize")}>
      {#if maximized}
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1">
          <rect x="0.5" y="2.5" width="7" height="7" /><path d="M2.5 2.5v-2h7v7h-2" />
        </svg>
      {:else}
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1">
          <rect x="0.5" y="0.5" width="9" height="9" />
        </svg>
      {/if}
    </button>
    <button class="wc close" onclick={() => win.close()} aria-label={t("win.close")} title={t("win.close")}>
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" stroke="currentColor" stroke-width="1"><path d="M0 0l10 10M10 0L0 10" /></svg>
    </button>
  </div>
</div>

<style>
  .chrome {
    height: 32px;
    flex-shrink: 0;
    display: flex;
    align-items: stretch;
    background: var(--rail);
    border-bottom: 1px solid var(--line);
    user-select: none;
    -webkit-user-select: none;
  }

  .grow {
    flex: 1;
  }

  .controls {
    display: flex;
  }

  /* Windows-sized hit targets, in our palette: no hover pill, a flat fill. */
  .wc {
    width: 46px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--dim);
  }

  .wc:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .wc.close:hover {
    color: var(--accon);
    background: var(--acc);
  }
</style>
