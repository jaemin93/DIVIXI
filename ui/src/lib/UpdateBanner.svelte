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
   * The whole of an update can be done from here: press "update" and the
   * download starts where you stand, the bytes are counted in the banner, and
   * what comes out the far side is the button that opens the verified
   * installer. Nobody is sent to settings to get it done -- the card is for
   * the detail (where the file is, its folder, the check's own history), so
   * "open settings" stays as a second, quieter link.
   *
   * Because both can start a download, both read and write the same store
   * fields, and `store.downloadUpdate` is the only way either of them starts
   * one -- it refuses while `updateProgress` is set, so two presses in two
   * places cannot become two downloads (src-tauri/src/update.rs keeps its own
   * BUSY flag behind that, for the same reason).
   *
   * Nothing in this component asks GitHub anything: it draws what the store
   * already knows, and the store knows it from a press or from the one check
   * at startup (ui/src/lib/updateSchedule.ts).
   */
  import { t } from "./i18n.svelte";
  import { local } from "./ipc.svelte";
  import { store, UPDATE_DOWNLOAD_WRONG, updateProgressText, updatePercent, updateWrongText } from "./store.svelte";

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

  /**
   * Whether the app fetches an installer for this platform at all. Windows
   * only today: `installer()` in src-tauri/src/update.rs is the one answer,
   * and `can_download` carries it here. Where it is false there is no button
   * to offer, and the release page stays the way through.
   */
  const canDownload = $derived(found?.can_download === true);

  /** A download of ours is running, so neither button starts a second one. */
  const busy = $derived(store.updateProgress !== null);

  /** How the download ended badly, when it did. */
  const wrong = $derived.by(() => {
    const got = store.updateGot;
    return got && UPDATE_DOWNLOAD_WRONG.has(got.kind) ? updateWrongText(got) : null;
  });

  /** Where the log and the card are, for the detail this banner leaves out. */
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
          <!-- The warning belongs wherever the installer can be opened, not
               only on the card. Someone who did the whole update from this
               banner never sees that card, and the unsigned-publisher dialog
               is the one thing they must not be surprised by. -->
          <p class="blurb warn">{t("settings.updateWarnUnsigned")}</p>
        {:else if state === "working"}
          <div class="title">{t("update.banner.working")}</div>
          <!-- The same line the card draws under its bar, from the same
               function: which phase, and how many bytes while they arrive. -->
          <p class="blurb mono">{store.updateProgress ? updateProgressText(store.updateProgress) : ""}</p>
          {#if store.updateProgress}
            {@const pct = updatePercent(store.updateProgress)}
            <div class="bar" class:indeterminate={pct === null}>
              <div class="fill" style="width: {pct ?? 30}%"></div>
            </div>
          {/if}
        {:else if state === "failed"}
          <div class="title">{t("update.banner.failed")}</div>
          <p class="blurb">{wrong?.say ?? t("settings.updateDownloadFailed")}</p>
          {#if wrong?.why}<p class="blurb why mono">{wrong.why}</p>{/if}
        {:else}
          <div class="title">{t("update.banner.found", { tag: found?.latest ?? "" })}</div>
          <p class="blurb">{t("update.banner.foundBlurb", { current: found?.current ?? "" })}</p>
          {#if !canDownload}
            <!-- No installer is fetched here, so there is no button to press
                 and the release page is the whole of it. -->
            <p class="blurb">{t("settings.updateManualOnly")}</p>
          {/if}
        {/if}
      </div>
      <div class="acts">
        {#if state === "ready"}
          <button class="btn btn-acc" type="button" onclick={() => store.openInstaller()}>{t("settings.updateOpenInstaller")}</button>
        {:else if state === "working"}
          <button class="btn" type="button" onclick={() => store.cancelUpdate()}>{t("settings.updateCancel")}</button>
        {:else if state === "failed"}
          <button class="btn btn-acc" type="button" disabled={busy} onclick={() => store.downloadUpdate()}>{t("settings.updateRetry")}</button>
        {:else if canDownload}
          <button class="btn btn-acc" type="button" disabled={busy} onclick={() => store.downloadUpdate()}>{t("update.banner.update")}</button>
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

  /* The unsigned-publisher note, in the colour the card gives the same
     sentence. */
  .blurb.warn {
    color: var(--warn);
  }

  /* A failure's own words under the sentence about it. */
  .blurb.why {
    font-size: 11px;
    color: var(--lab);
    word-break: break-word;
  }

  /* The same bar as the settings card, at the same 3px. */
  .bar {
    margin-top: 7px;
    height: 3px;
    background: var(--line);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--acc);
    transition: width 120ms linear;
  }

  /* No total to draw a fraction of: a sliver that moves, and the amount in
     words above it. */
  .indeterminate .fill {
    animation: slide 1.2s ease-in-out infinite;
  }

  @keyframes slide {
    0% {
      transform: translateX(-100%);
    }
    100% {
      transform: translateX(340%);
    }
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
