<script lang="ts">
  /**
   * A release is out, or its installer is being fetched, or one is verified
   * and waiting to be opened — said once, over every view.
   *
   * Not a toast. An error toast is for something that just failed in front of
   * the human and goes by itself; an update is not urgent and not over, and
   * the useful thing to do about it takes longer than four seconds. So it
   * stays until it is closed, like CrashBanner beside it.
   *
   * It says the state and offers the two presses that matter here; everything
   * else — the path, the hashes, the failures with their reasons — is on the
   * settings card, which the button opens. Nothing in this component asks
   * GitHub anything: it draws what the store already knows, and the store only
   * knows it because someone pressed a button (src-tauri/src/update.rs).
   *
   * There is nothing to show on a launch today: no check runs at startup. Once
   * `store.checkAtStartup` is wired up, this is where the answer surfaces.
   */
  import { t } from "./i18n.svelte";
  import { local } from "./ipc.svelte";
  import { store, UPDATE_DOWNLOAD_WRONG, updateProgressText } from "./store.svelte";

  /** Which of the four things it has to say, or null for none of them. */
  const state = $derived.by<"found" | "working" | "ready" | "failed" | null>(() => {
    // This PC's own app is the one an update replaces; an instance is updated
    // where it runs, as its settings card says.
    if (!local || !store.updateWorthSaying) return null;
    if (store.updateProgress !== null) return "working";
    const got = store.updateGot;
    if (got?.kind === "ready") return "ready";
    if (got && UPDATE_DOWNLOAD_WRONG.has(got.kind)) return "failed";
    return store.updateCheck?.kind === "update" ? "found" : null;
  });

  /** The release found, when one was. */
  const found = $derived(store.updateCheck?.kind === "update" ? store.updateCheck : null);

  /** Where the log and the card are, which is the useful next step. */
  function toUpdates() {
    store.openSettings("about");
    store.updateBannerClosed = true;
  }
</script>

<!-- The region is in the page from the start, empty or not: a screen reader
     announces what is put *into* a live region, and this fills in a moment
     after a press somewhere else. -->
<div class="spot" class:empty={state === null} role="status" aria-live="polite" aria-atomic="true">
  {#if state !== null}
    <div class="banner" class:bad={state === "failed"}>
      <span class="mark" aria-hidden="true">↑</span>
      <div class="says">
        {#if state === "ready"}
          <div class="title">{t("update.banner.ready")}</div>
          <p class="blurb">{t("update.banner.readyBlurb", { name: store.updateReady?.name ?? "" })}</p>
        {:else if state === "working"}
          <div class="title">{t("update.banner.working")}</div>
          <!-- The same line the card draws under its bar, from the same
               function: which phase, and how many bytes while they arrive. -->
          <p class="blurb mono">{store.updateProgress ? updateProgressText(store.updateProgress) : ""}</p>
        {:else if state === "failed"}
          <div class="title">{t("update.banner.failed")}</div>
          <p class="blurb">{t("settings.updateDownloadFailed")}</p>
        {:else}
          <div class="title">{t("update.banner.found", { tag: found?.latest ?? "" })}</div>
          <p class="blurb">{t("update.banner.foundBlurb", { current: found?.current ?? "" })}</p>
        {/if}
      </div>
      <div class="acts">
        {#if state === "ready"}
          <button class="btn btn-acc" type="button" onclick={() => store.openInstaller()}>{t("settings.updateOpenInstaller")}</button>
        {/if}
        <button class="btn" type="button" onclick={toUpdates}>{t("update.banner.details")}</button>
        <button class="close" type="button" onclick={() => (store.updateBannerClosed = true)} title={t("update.banner.dismiss")} aria-label={t("update.banner.dismiss")}>✕</button>
      </div>
    </div>
  {/if}
</div>

<style>
  /* Under the window chrome, across the middle, like CrashBanner — and below
     it in the stack, because a crash is the thing to read first if both
     happen to be up. */
  .spot {
    position: fixed;
    top: 42px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 205;
    max-width: min(560px, calc(100vw - 36px));
  }

  .spot.empty {
    display: none;
  }

  .banner {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 14px;
    background: var(--card);
    border: 1px solid var(--line);
    border-left: 2px solid var(--acc);
    box-shadow: 0 4px 18px rgb(0 0 0 / 0.35);
  }

  /* A download that failed is not news to be pleased about. */
  .banner.bad {
    border-left-color: var(--acct);
  }

  .mark {
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    margin-top: 2px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--acc);
    color: var(--accon);
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1;
  }

  .banner.bad .mark {
    background: var(--acct);
  }

  .says {
    flex: 1;
    min-width: 0;
  }

  .title {
    font-size: 13.5px;
    color: var(--hi);
  }

  .blurb {
    margin: 5px 0 0;
    font-size: 12px;
    line-height: 1.55;
    color: var(--dim);
  }

  .acts {
    flex-shrink: 0;
    display: flex;
    align-items: flex-start;
    gap: 6px;
  }

  .close {
    width: 20px;
    height: 20px;
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--lab);
    font-size: 11px;
    line-height: 1;
  }

  .close:hover {
    color: var(--hi);
  }
</style>
