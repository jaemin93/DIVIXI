<script lang="ts">
  import { t } from "./i18n.svelte";
  import { isFindKey } from "./boardFind";
  import { countLabel, type FindOptions } from "./chatFind";

  /**
   * The conversation's find bar, after the design board's: the words, how
   * many were found and which one is shown, the way up and down, and close.
   * Enter and Shift+Enter (or the arrow keys) step, Esc closes. The toggles
   * say how to compare (case, whole words) and whether folded steps count.
   * What to find and where it is shown are the timeline's (Timeline.svelte).
   */
  let {
    query = $bindable(""),
    opts = $bindable(),
    at,
    count,
    onstep,
    onclose,
  }: {
    query?: string;
    opts: FindOptions;
    /** The current match, from 0; -1 for none. */
    at: number;
    count: number;
    onstep: (back: boolean) => void;
    onclose: () => void;
  } = $props();

  const mac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  let input = $state<HTMLInputElement>();
  const label = $derived(countLabel(at, count));

  /** Put the cursor in the box, its words selected for typing over. */
  export function focus() {
    input?.focus();
    input?.select();
  }

  function onKey(e: KeyboardEvent) {
    // A Korean syllable still being composed takes its Enter itself.
    if (e.isComposing) return;
    if (e.key === "Enter" || e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      onstep(e.key === "ArrowUp" || (e.key === "Enter" && e.shiftKey));
    } else if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    } else if (isFindKey(e, mac)) {
      e.preventDefault();
      input?.select();
    }
  }
</script>

<div class="find" role="search" aria-label={t("find.label")}>
  <input
    bind:this={input}
    bind:value={query}
    type="text"
    spellcheck="false"
    autocomplete="off"
    placeholder={t("find.placeholder")}
    aria-label={t("find.label")}
    onkeydown={onKey}
  />
  <!-- One group: on a narrow column it moves under the box as a whole. -->
  <span class="ctrls">
  <span class="mono fcount" aria-live="polite">{query.trim() ? (label ? t("find.count", label) : t("find.none")) : ""}</span>
  <button type="button" class="opt mono" class:on={opts.matchCase} aria-pressed={opts.matchCase} onclick={() => (opts = { ...opts, matchCase: !opts.matchCase })} title={t("find.matchCase")} aria-label={t("find.matchCase")}>Aa</button>
  <button type="button" class="opt mono word" class:on={opts.wholeWord} aria-pressed={opts.wholeWord} onclick={() => (opts = { ...opts, wholeWord: !opts.wholeWord })} title={t("find.wholeWord")} aria-label={t("find.wholeWord")}>ab</button>
  <button type="button" class="opt mono steps" class:on={opts.steps} aria-pressed={opts.steps} onclick={() => (opts = { ...opts, steps: !opts.steps })} title={t("find.stepsTitle")} aria-label={t("find.stepsTitle")}>{t("find.steps")}</button>
  <span class="sep" aria-hidden="true"></span>
  <button type="button" disabled={!count} onclick={() => onstep(true)} aria-label={t("find.prev")} title={t("find.prev")}>↑</button>
  <button type="button" disabled={!count} onclick={() => onstep(false)} aria-label={t("find.next")} title={t("find.next")}>↓</button>
  <button type="button" onclick={onclose} aria-label={t("find.close")} title={t("find.close")}>×</button>
  </span>
</div>

<style>
  /* The design board's find bar (Board.svelte .find), over the top of the conversation. */
  .find {
    position: absolute;
    top: 10px;
    right: 34px;
    z-index: 6;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    /* Room for the box and every control on one row; a narrow column (a
       phone, an agent panel) puts the controls on a second row instead of
       squeezing the box. */
    width: min(560px, calc(100% - 24px));
    padding: 4px;
    background: var(--card);
    border: 1px solid var(--lines);
    box-shadow: 0 2px 10px rgb(0 0 0 / 0.28);
  }

  .find input {
    flex: 1 1 160px;
    min-width: 0;
    height: 28px;
    padding: 0 8px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-size: 12px;
  }

  .find input:focus {
    border-color: var(--acc);
  }

  .ctrls {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 0 0 auto;
    margin-left: auto;
  }

  .fcount {
    min-width: 54px;
    text-align: center;
    font-size: 10px;
    color: var(--lab);
    white-space: nowrap;
  }

  .find button {
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--dim);
  }

  .find button:hover:not(:disabled) {
    color: var(--hi);
    background: var(--sel);
  }

  .find button:disabled {
    opacity: 0.4;
  }

  /* A toggle: quiet when off, the accent's outline when on. */
  .find .opt {
    font-size: 11px;
    border: 1px solid transparent;
  }

  .find .opt.word {
    text-decoration: underline;
    text-underline-offset: 3px;
  }

  .find .opt.steps {
    width: auto;
    padding: 0 6px;
    font-size: 10px;
  }

  .find .opt.on {
    color: var(--hi);
    border-color: var(--acc);
  }

  .sep {
    width: 1px;
    height: 18px;
    margin: 0 2px;
    background: var(--line);
    flex-shrink: 0;
  }

  /* A phone: the bar takes the width, the box the first row. */
  @media (max-width: 640px) {
    .find {
      left: 8px;
      right: 8px;
      top: 8px;
      width: auto;
    }

    .find input {
      flex-basis: 100%;
    }
  }
</style>
