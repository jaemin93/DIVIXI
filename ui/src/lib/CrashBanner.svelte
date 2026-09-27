<script lang="ts">
  /**
   * The run before this one stopped on a panic, and says so once.
   *
   * Not a toast. An error toast is for something that just failed in front of
   * the human and goes by itself; this is about a run that is already over,
   * and the one thing worth doing about it — sending the log — is not
   * something anyone does in four seconds. So it stays until it is closed.
   *
   * Not the notification bell either: the bell is for decisions waiting on
   * an answer.
   *
   * It does not come back. `last_crash` answers for the run that just ended
   * and nothing else (src-tauri/src/crash.rs), so closing it here is enough:
   * the next launch has nothing to report unless this one panics too. That is
   * also why there is nothing to remember in the store.
   */
  import { onMount } from "svelte";

  import { t } from "./i18n.svelte";
  import { invoke, local } from "./ipc.svelte";
  import { store } from "./store.svelte";

  /** Mirrors `crash::Crash`. */
  type Crash = { when: string; message: string; location: string; detail: string; path: string };

  let crash = $state<Crash | null>(null);

  onMount(() => {
    // This PC's own log; a remote instance has no panic of ours to report.
    if (!local) return;
    invoke<Crash | null>("last_crash")
      .then((c) => (crash = c))
      .catch(() => {});
  });

  /** Where the log and the report are, which is the only useful next step. */
  function toDiagnostics() {
    store.openSettings("about");
    crash = null;
  }
</script>

<!-- The region is in the page from the start, empty or not: a screen reader
     announces what is put *into* a live region, and `last_crash` answers a
     moment after this is mounted. -->
<div class="spot" class:empty={!crash} role="alert" aria-live="assertive" aria-atomic="true">
  {#if crash}
    <div class="banner">
      <span class="mark" aria-hidden="true">!</span>
      <div class="says">
        <div class="title">{t("crash.title")}</div>
        <p class="blurb">{t("crash.blurb")}</p>
        <p class="detail mono">{crash.message}</p>
        <p class="stamp mono">{t("crash.stamp", { when: crash.when, where: crash.location })}</p>
      </div>
      <div class="acts">
        <button class="btn" type="button" onclick={toDiagnostics}>{t("crash.open")}</button>
        <button class="close" type="button" onclick={() => (crash = null)} title={t("crash.dismiss")} aria-label={t("crash.dismiss")}>✕</button>
      </div>
    </div>
  {/if}
</div>

<style>
  /* Under the window chrome (32px), across the middle: the first thing read
     on a launch that follows a crash, and over every view because it belongs
     to none of them. */
  .spot {
    position: fixed;
    top: 42px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 210;
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
    background: var(--warnbg);
    border: 1px solid var(--warnln);
    border-left: 2px solid var(--warn);
    box-shadow: 0 4px 18px rgb(0 0 0 / 0.35);
  }

  .mark {
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    margin-top: 2px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--warn);
    color: var(--bg);
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1;
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

  /* The panic's own words, and where it was. Two lines of it is enough to
     recognise; the whole block is in the file the report names. */
  .detail {
    margin: 8px 0 0;
    font-size: 11px;
    line-height: 1.5;
    color: var(--txt);
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
  }

  .stamp {
    margin: 4px 0 0;
    font-size: 10px;
    color: var(--lab);
    overflow-wrap: anywhere;
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
