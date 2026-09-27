<script lang="ts">
  /**
   * Everything that went wrong, wherever it went wrong.
   *
   * One stack, mounted once above every view, so an error raised in
   * settings or the workspace panel is as visible as one raised in the
   * conversation. Each goes by itself after a while (`errorLife`), or the
   * moment the human closes it. Hovering or tabbing into the stack stops
   * every clock: an error is never taken away while it is being read.
   *
   * Not the notification bell. The bell is for decisions waiting on an
   * answer; these are things that already failed and only need reading.
   */
  import { store, errorLife } from "./store.svelte";
  import { t } from "./i18n.svelte";

  /** While the human is on the stack, nothing expires. */
  let held = $state(false);

  /** Leaving the stack does not fire every overdue clock at once. */
  const GRACE = 1200;
  let wasHeld = false;

  // One timer per error, cleared when it goes or when the stack is held.
  // The deadlines are absolute, so re-running this (another error arrived,
  // one was closed) never lengthens anyone's stay — except just after the
  // human lets go of the stack, where everything overdue gets a moment.
  $effect(() => {
    if (held) {
      wasHeld = true;
      return;
    }
    const grace = wasHeld ? GRACE : 0;
    wasHeld = false;
    const now = Date.now();
    const timers = store.errors.map((e) =>
      setTimeout(() => store.dismissError(e.id), Math.max(grace, e.at + errorLife(e.text) - now)),
    );
    return () => timers.forEach(clearTimeout);
  });
</script>

<!-- The region is always here, empty or not: a screen reader announces what
     is put *into* a live region, and one that appears along with its first
     message is easy for it to miss.
     `aria-atomic="false"` with `aria-relevant="additions"` keeps it to the
     error that just arrived — not the whole stack re-read every time, and
     not a word when one quietly expires. -->
<div
  class="stack"
  class:empty={!store.errors.length}
  role="alert"
  aria-live="assertive"
  aria-atomic="false"
  aria-relevant="additions"
  onmouseenter={() => (held = true)}
  onmouseleave={() => (held = false)}
  onfocusin={() => (held = true)}
  onfocusout={() => (held = false)}
>
  {#each store.errors as e (e.id)}
    <div class="toast">
      <span class="mark" aria-hidden="true">!</span>
      <span class="text">{e.text}</span>
      {#if e.again}<span class="mono again" title={t("error.againTitle")}>×{e.again + 1}</span>{/if}
      <button class="close" type="button" onclick={() => store.dismissError(e.id)} title={t("error.dismiss")} aria-label={t("error.dismiss")}>✕</button>
    </div>
  {/each}
  {#if store.errors.length > 1}
    <button class="clear mono" type="button" onclick={() => store.dismissErrors()}>{t("error.dismissAll")}</button>
  {/if}
</div>

<style>
  /* Over everything, out of the way of the composer and the rail. The box
     is only ever as tall as the toasts in it, so what it covers is what
     they cover; it takes pointer events because past its ceiling it has
     to be scrollable. */
  .stack {
    position: fixed;
    right: 18px;
    bottom: 18px;
    z-index: 200;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
    max-width: min(440px, calc(100vw - 36px));
    /* Never more than half the screen, however fast they arrive. The
       stack scrolls once it is full, so it takes its own pointer events
       back — the toasts inside already have theirs. */
    max-height: 50vh;
    overflow-y: auto;
    pointer-events: auto;
  }

  /* Nothing to show, but the live region stays in the page. */
  .stack.empty {
    display: none;
  }

  .toast {
    pointer-events: auto;
    display: flex;
    align-items: flex-start;
    gap: 10px;
    width: 100%;
    padding: 10px 12px;
    background: var(--card);
    border: 1px solid var(--acc);
    border-left-width: 2px;
    box-shadow: 0 3px 14px rgb(0 0 0 / 0.3);
  }

  .mark {
    flex-shrink: 0;
    width: 16px;
    height: 16px;
    margin-top: 1px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--acc);
    color: var(--accon);
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1;
  }

  .text {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--txt);
    /* A message from a process can be one very long word (a path). */
    overflow-wrap: anywhere;
    /* Six lines is plenty; the whole of it is in the webview console. */
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 6;
    line-clamp: 6;
    overflow: hidden;
  }

  .again {
    flex-shrink: 0;
    font-size: 9px;
    letter-spacing: 0.1em;
    color: var(--lab);
    border: 1px solid var(--line);
    padding: 1px 5px;
  }

  .close {
    flex-shrink: 0;
    width: 18px;
    height: 18px;
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

  .clear {
    pointer-events: auto;
    height: 24px;
    padding: 0 10px;
    background: var(--card);
    border: 1px solid var(--line);
    color: var(--dim);
    font-size: 9px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
  }

  .clear:hover {
    color: var(--hi);
    background: var(--sel);
  }
</style>
