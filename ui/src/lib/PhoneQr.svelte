<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { t } from "./i18n.svelte";
  import { qrPath } from "./qr";
  import { pairing } from "./pairing.svelte";

  /**
   * A pairing code, drawn and counted down.
   *
   * One component for both places that show one — the Overview card and the
   * rail's dialog — and the code itself lives in `pairing`, outside this
   * component, so closing one of them does not throw away a code that still
   * works and opening the other does not mint a second.
   */

  let { onhide }: { onhide?: () => void } = $props();

  let copied = $state(false);
  let ticking: ReturnType<typeof setInterval> | undefined;

  onMount(() => {
    void pairing.ensure();
    ticking = setInterval(() => pairing.tick(), 1000);
  });
  onDestroy(() => clearInterval(ticking));

  /**
   * How wide to draw it, in whole pixels per module.
   *
   * A fixed width was wrong at both ends: a realistic pairing URL is a
   * version 11 symbol, 61 modules across, and 148px of it is 2.3 pixels a
   * module — under what a camera can resolve off a screen. Four is the usual
   * floor. Sizing from the module count also keeps every module a whole
   * number of pixels, so the browser is not resampling module edges.
   */
  const PX_PER_MODULE = 4;
  const across = $derived(pairing.qr ? pairing.qr.size + 4 : 0);
  const width = $derived(across * PX_PER_MODULE);

  async function copy() {
    if (!pairing.link) return;
    await navigator.clipboard.writeText(pairing.link.url);
    copied = true;
    setTimeout(() => (copied = false), 1600);
  }

  function hide() {
    // The code is left standing: it is still good, and showing this again
    // should bring back the same one rather than mint another.
    onhide?.();
  }
</script>

<div class="qrwrap">
  {#if pairing.error}
    <p class="warn">{pairing.error}</p>
    <button class="btn sm" onclick={() => pairing.mint()}>{t("phone.qr.again")}</button>
  {:else if pairing.qr && pairing.link}
    <!-- One path for the whole symbol: a version 11 code is over three
         thousand modules, and that many elements is slow to lay out.
         `shape-rendering` keeps the module edges hard. -->
    <svg
      class="qr"
      style="width: {width}px; height: {width}px"
      viewBox="0 0 {across} {across}"
      shape-rendering="crispEdges"
      role="img"
      aria-label={t("phone.qr.alt")}
    >
      <rect width={across} height={across} fill="#fff" />
      <path d={qrPath(pairing.qr, 2)} fill="#000" />
    </svg>

    <hr class="rule" />

    <p class="warn">{t("phone.qr.warn", { clock: pairing.clock, days: String(pairing.link.days) })}</p>

    <div class="row">
      <button class="btn sm" onclick={copy}>{copied ? t("phone.copied") : t("phone.qr.copyLink")}</button>
      <button class="btn sm" onclick={hide}>{t("phone.qr.hide")}</button>
    </div>
  {:else}
    <p class="hint">{t("phone.qr.making")}</p>
  {/if}
</div>

<style>
  .qrwrap {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: flex-start;
  }

  /* White behind the code whatever the theme: a reader needs the contrast,
     and an inverted code is not a code. */
  .qr {
    background: #fff;
  }

  .rule {
    width: 100%;
    height: 0;
    margin: 2px 0;
    border: 0;
    border-top: 1px solid var(--line);
  }

  .row {
    display: flex;
    gap: 8px;
  }

  .warn {
    margin: 0;
    max-width: 34rem;
    font-size: 12px;
    line-height: 1.55;
    color: var(--warn);
  }

  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--dim);
  }
</style>
