<script lang="ts">
  import { store, type Decision } from "./store.svelte";
  import { t } from "./i18n.svelte";
  import { whenFull, whenLabel } from "./time";

  /**
   * A question the conductor put to the human, in the timeline where it was
   * asked. Open: one button per option, the recommended one marked, an
   * answer in one's own words when allowed, an optional note. Answered: the
   * choice, quietly. The answer goes back to the conductor as a turn.
   */
  let { decision }: { decision: Decision } = $props();

  const OWN = -1;
  let picked = $state<number | null>(null);
  let own = $state("");
  let note = $state("");
  let noteOpen = $state(false);

  const letter = (i: number) => String.fromCharCode(65 + (i % 26));
  const open = $derived(decision.status === "open");
  const sending = $derived(store.answering === decision.id);
  const ready = $derived(picked !== null && (picked !== OWN || own.trim().length > 0) && !sending);
  /** What the human chose, as the answered card shows it. */
  const chosen = $derived(decision.choice === null ? null : decision.options[decision.choice]);

  async function submit() {
    if (!ready || picked === null) return;
    const choice = picked === OWN ? null : picked;
    await store.answerDecision(decision.id, choice, picked === OWN ? own : null, note);
  }

  function onOwnKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void submit();
    }
  }
</script>

<section class="decision" class:open class:dismissed={decision.status === "dismissed"}>
  <div class="head">
    <span class="mlab kind">{t("decision.label")} · #{decision.id}</span>
    {#if open}
      <span class="mono state waiting">{t("decision.waiting")}</span>
    {:else if decision.status === "decided"}
      <span class="mono state done">✓ {t("decision.decided")}</span>
    {:else}
      <span class="mono state">{t("decision.dismissed")}</span>
    {/if}
    <span class="grow"></span>
    <span class="mono when" title={whenFull(decision.decided_at ?? decision.created_at, store.lang)}>
      {whenLabel(decision.decided_at ?? decision.created_at, store.now, store.lang)}
    </span>
  </div>

  <p class="q">{decision.question}</p>

  {#if open}
    {#if decision.context}<p class="ctx">{decision.context}</p>{/if}

    <div class="opts" role="radiogroup" aria-label={decision.question}>
      {#each decision.options as option, i (i)}
        <button class="opt" class:on={picked === i} role="radio" aria-checked={picked === i} onclick={() => (picked = i)} disabled={sending}>
          <span class="key mono">{letter(i)}</span>
          <span class="body">
            <span class="label">
              {option.label}
              {#if decision.recommended === i}<span class="rec mono">{t("decision.recommended")}</span>{/if}
            </span>
            {#if option.detail}<span class="detail">{option.detail}</span>{/if}
          </span>
        </button>
      {/each}
      {#if decision.allow_other}
        <button class="opt" class:on={picked === OWN} role="radio" aria-checked={picked === OWN} onclick={() => (picked = OWN)} disabled={sending}>
          <span class="key mono">…</span>
          <span class="body"><span class="label">{t("decision.other")}</span></span>
        </button>
        {#if picked === OWN}
          <!-- svelte-ignore a11y_autofocus -->
          <textarea class="own" rows="2" bind:value={own} onkeydown={onOwnKey} placeholder={t("decision.otherPlaceholder")} autofocus disabled={sending}></textarea>
        {/if}
      {/if}
    </div>

    <div class="foot">
      {#if noteOpen}
        <input class="note" type="text" bind:value={note} placeholder={t("decision.notePlaceholder")} maxlength="400" disabled={sending} />
      {:else}
        <button class="link mono" onclick={() => (noteOpen = true)}>{t("decision.addNote")}</button>
      {/if}
      <span class="grow"></span>
      <button class="btn" onclick={() => store.dismissDecision(decision.id)} disabled={sending}>{t("decision.skip")}</button>
      <button class="btn btn-acc" onclick={submit} disabled={!ready}>{sending ? t("decision.sending") : t("decision.submit")}</button>
    </div>
  {:else if decision.status === "decided"}
    <div class="answer">
      {#if chosen && decision.choice !== null}
        <span class="key mono on">{letter(decision.choice)}</span>
        <span class="label">{chosen.label}</span>
      {:else}
        <span class="key mono on">…</span>
        <span class="label">{decision.answer ?? ""}</span>
      {/if}
    </div>
    {#if decision.note}<p class="ctx">{decision.note}</p>{/if}
    {#if decision.options.length > 1}
      <div class="rest mono">
        {t("decision.others")}:
        {#each decision.options as option, i (i)}
          {#if i !== decision.choice}<span class="restopt">{letter(i)}. {option.label}</span>{/if}
        {/each}
      </div>
    {/if}
  {/if}
</section>

<style>
  /* Orange means waiting on the human; once answered the card goes quiet. */
  .decision {
    max-width: 680px;
    margin: -10px 0 22px;
    padding: 12px 17px 14px;
    border: 1px solid var(--line);
    border-left: 2px solid var(--lines);
    background: var(--card);
  }

  .decision.open {
    border-color: var(--warnln);
    border-left-color: var(--warn);
    background: var(--warnbg);
  }

  .decision.dismissed {
    opacity: 0.6;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .open .kind {
    color: var(--warn);
  }

  .state {
    font-size: 9px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--lab);
  }

  .state.waiting {
    color: var(--warn);
  }

  .state.done {
    color: var(--ok);
  }

  .when {
    font-size: 9px;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .q {
    margin: 8px 0 0;
    font-family: var(--sans);
    font-size: calc(var(--chat-fs) + 1px);
    line-height: 1.5;
    color: var(--hi);
    white-space: pre-wrap;
  }

  :not(.open) > .q {
    font-size: var(--chat-fs);
    color: var(--txt);
  }

  .ctx {
    margin: 6px 0 0;
    font-family: var(--sans);
    font-size: calc(var(--chat-fs) - 1px);
    line-height: 1.6;
    color: var(--dim);
    white-space: pre-wrap;
  }

  .opts {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 12px;
  }

  .opt {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    width: 100%;
    padding: 9px 12px;
    background: var(--card);
    border: 1px solid var(--line);
    text-align: left;
    color: var(--txt);
    font-family: var(--sans);
  }

  .opt:hover:not(:disabled) {
    border-color: var(--lines);
    background: var(--sel);
  }

  .opt.on {
    border-color: var(--warn);
    background: var(--sel);
  }

  .key {
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--lines);
    font-size: 10px;
    color: var(--lab);
  }

  .opt.on .key,
  .key.on {
    border-color: var(--warn);
    background: var(--warn);
    color: var(--bg);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .label {
    font-size: var(--chat-fs);
    line-height: 20px;
    color: var(--hi);
  }

  .detail {
    font-size: calc(var(--chat-fs) - 1px);
    line-height: 1.55;
    color: var(--dim);
  }

  .rec {
    margin-left: 8px;
    padding: 1px 5px;
    border: 1px solid var(--warnln);
    font-size: 9px;
    letter-spacing: 0.12em;
    color: var(--warn);
    vertical-align: 1px;
  }

  .own,
  .note {
    width: 100%;
    padding: 8px 10px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-family: var(--sans);
    font-size: var(--chat-fs);
    outline: none;
    resize: vertical;
  }

  .own:focus,
  .note:focus {
    border-color: var(--warn);
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
  }

  .note {
    flex: 1;
    height: 32px;
    padding: 0 10px;
  }

  .link {
    padding: 0;
    background: transparent;
    border: 0;
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--lab);
  }

  .link:hover {
    color: var(--hi);
  }

  .answer {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 8px;
  }

  .answer .key.on {
    border-color: var(--ok);
    background: transparent;
    color: var(--ok);
  }

  .rest {
    margin-top: 8px;
    font-size: 10px;
    color: var(--lab);
  }

  .restopt {
    margin-left: 10px;
  }
</style>
