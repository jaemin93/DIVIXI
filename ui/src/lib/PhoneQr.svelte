<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "./ipc.svelte";
  import { t } from "./i18n.svelte";
  import { encodeQr, qrPath, type Qr } from "./qr";

  /**
   * A pairing code, drawn and counted down.
   *
   * One component for both places that show one — the Overview card and the
   * rail's modal — so there is one mint, one countdown and one warning. Two
   * copies of this would be two chances for one of them to leave a dead code
   * on screen.
   *
   * The code is taken off the screen the moment it expires rather than left
   * sitting there: a code that no longer works looks exactly like one that
   * does, and the only way to find out is to fail at the phone.
   */

  let { onhide }: { onhide?: () => void } = $props();

  type PairLink = { url: string; expires: number; days: number };

  let link = $state<PairLink | null>(null);
  let qr = $state<Qr | null>(null);
  let error = $state("");
  let copied = $state(false);
  let left = $state(0);

  let ticking: ReturnType<typeof setInterval> | undefined;

  async function mint() {
    error = "";
    link = null;
    qr = null;
    try {
      const got = await invoke<PairLink>("phone_pair_link");
      // Drawn before it is shown: a payload too long to encode must fail here
      // and not as an empty square on the screen.
      qr = encodeQr(got.url);
      link = got;
      tick();
    } catch (err) {
      error = String(err);
    }
  }

  function tick() {
    if (!link) return;
    left = Math.max(0, link.expires - Math.floor(Date.now() / 1000));
    if (left === 0) {
      // Expired. Nothing to copy and nothing to scan, so nothing is shown.
      link = null;
      qr = null;
    }
  }

  onMount(() => {
    void mint();
    ticking = setInterval(tick, 1000);
  });
  onDestroy(() => clearInterval(ticking));

  const clock = $derived(`${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`);

  async function copy() {
    if (!link) return;
    await navigator.clipboard.writeText(link.url);
    copied = true;
    setTimeout(() => (copied = false), 1600);
  }

  function hide() {
    link = null;
    qr = null;
    onhide?.();
  }
</script>

<div class="qrwrap">
  {#if error}
    <p class="warn">{error}</p>
    <button class="btn sm" onclick={mint}>{t("phone.qr.again")}</button>
  {:else if qr && link}
    <!-- One path for the whole symbol: a version 12 code is over two thousand
         modules, and that many elements is slow to lay out. `shape-rendering`
         keeps the edges hard when the browser scales it. -->
    <svg
      class="qr"
      viewBox="0 0 {qr.size + 4} {qr.size + 4}"
      shape-rendering="crispEdges"
      role="img"
      aria-label={t("phone.qr.alt")}
    >
      <rect width={qr.size + 4} height={qr.size + 4} fill="#fff" />
      <path d={qrPath(qr, 2)} fill="#000" />
    </svg>

    <p class="warn">{t("phone.qr.warn", { clock, days: String(link.days) })}</p>

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
    width: 200px;
    height: 200px;
    background: #fff;
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
