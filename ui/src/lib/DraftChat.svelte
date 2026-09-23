<script lang="ts">
  import { store, type Segment } from "./store.svelte";
  import Markdown from "./Markdown.svelte";
  import Working from "./Working.svelte";
  import { boardPng } from "./ink";
  import { t } from "./i18n.svelte";

  /**
   * Talking with the draft's agent, beside the board. Every message carries
   * the board's outline, a picture of it when it has ink, and the items
   * selected on it. The agent's board changes wait above the box to be
   * kept or reverted all at once (or one by one on the board).
   */
  let { selected = [] }: { selected?: string[] } = $props();

  let draft = $state("");
  let scroller = $state<HTMLDivElement>();
  const runs = $derived(store.draftRuns);
  const changes = $derived(store.draftDoc.changes);

  $effect(() => {
    void runs.length;
    void runs.at(-1)?.message.length;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });

  function send(e?: Event) {
    e?.preventDefault();
    if (store.draftBusy) return;
    const text = draft.trim();
    if (!text && !selected.length) return;
    draft = "";
    void store.draftSend(text, boardPng(store.draftDoc), selected);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      send();
    }
  }

  /** Board tools read as what they did, not as tool names. */
  function toolWord(title: string): string {
    const name = title.replace(/^mcp__[a-z0-9_-]+__/i, "").replace(/^mcp\.[a-z0-9_-]+\./i, "");
    if (name.startsWith("board_write")) return t("draft.toolWrite");
    if (name.startsWith("board_read")) return t("draft.toolRead");
    return name;
  }

  function texts(segments: Segment[]): string {
    return segments
      .filter((s) => s.kind === "text")
      .map((s) => (s.kind === "text" ? s.text : ""))
      .join("");
  }
</script>

<aside class="chat">
  <div class="scroll" bind:this={scroller}>
    {#if runs.length === 0}
      <p class="empty">{t("draft.chatEmpty")}</p>
    {/if}
    {#each runs as run (run.id)}
      {#if run.prompt.trim()}
        <div class="me"><p>{run.prompt}</p></div>
      {/if}
      <div class="agent" class:live={run.status === "running" || run.status === "connecting"}>
        {#each run.segments.filter((s) => s.kind === "tool") as seg, i (i)}
          {#if seg.kind === "tool"}
            <div class="mono toolline" class:bad={seg.tool.status === "failed"}>· {toolWord(seg.tool.title)}</div>
          {/if}
        {/each}
        {#if texts(run.segments).trim()}
          <div class="body"><Markdown source={texts(run.segments)} /></div>
        {:else if run.segments.length === 0 && run.message.trim()}
          <div class="body"><Markdown source={run.message} /></div>
        {/if}
        {#if run.status === "failed" && run.error}
          <p class="bad">{run.error}</p>
        {:else if run.status === "running" || run.status === "connecting"}
          <Working />
        {/if}
      </div>
    {/each}
  </div>

  {#if changes.length}
    <!-- The agent's board changes, all at once. -->
    <div class="review">
      <span class="mono">{t("draft.pending", { n: changes.length })}</span>
      <span class="grow"></span>
      <button type="button" class="btn sm" onclick={() => store.draftReview(changes.map((c) => c.id), false)}>{t("draft.revertAll")}</button>
      <button type="button" class="btn sm keep" onclick={() => store.draftReview(changes.map((c) => c.id), true)}>{t("draft.keepAll")}</button>
    </div>
  {/if}

  <form onsubmit={send}>
    {#if selected.length}
      <div class="mono picked">{t("draft.withSelected", { n: selected.length })}</div>
    {/if}
    <div class="row">
      <textarea
        bind:value={draft}
        rows="3"
        placeholder={t("draft.placeholder")}
        aria-label={t("draft.placeholder")}
        onkeydown={onKey}
        disabled={store.draftBusy}
      ></textarea>
      {#if store.draftBusy}
        <button class="btn send stop" type="button" onclick={() => store.draftCancel()} aria-label={t("composer.stop")}>■</button>
      {:else}
        <button class="btn send" type="submit" disabled={!draft.trim() && !selected.length} aria-label={t("composer.send")}>→</button>
      {/if}
    </div>
    {#if store.lastError}<div class="mono err" title={store.lastError}>{store.lastError}</div>{/if}
  </form>
</aside>

<style>
  .chat {
    width: 360px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 1px solid var(--line);
    background: var(--rail);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px 16px 8px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .empty {
    margin: 20px 4px;
    font-size: 13px;
    line-height: 1.6;
    color: var(--lab);
  }

  .me {
    align-self: flex-end;
    max-width: 90%;
    padding: 9px 12px;
    border: 1px solid var(--accln);
    border-bottom: 2px solid var(--acc);
    background: var(--accbg);
  }

  .me p {
    margin: 0;
    font-size: 13px;
    line-height: 1.6;
    color: var(--txt);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .agent {
    padding: 2px 0 2px 12px;
    border-left: 2px solid var(--lines);
  }

  .agent.live {
    border-left-color: var(--ok);
  }

  .body {
    font-size: 13px;
    line-height: 1.6;
    color: var(--txt);
  }

  .toolline {
    font-size: 10px;
    letter-spacing: 0.06em;
    color: var(--lab);
    margin-bottom: 4px;
  }

  .toolline.bad,
  .bad {
    color: var(--acct);
  }

  .bad {
    margin: 4px 0 0;
    font-size: 12px;
  }

  .review {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border-top: 1px solid var(--accln);
    background: var(--accbg);
    font-size: 10px;
    letter-spacing: 0.08em;
    color: var(--acct);
  }

  .grow {
    flex: 1;
  }

  .btn.sm {
    height: 26px;
    padding: 0 10px;
  }

  .btn.keep {
    border-color: var(--acc);
    color: var(--hi);
  }

  form {
    padding: 10px 12px 12px;
    border-top: 1px solid var(--line);
  }

  .picked {
    font-size: 10px;
    letter-spacing: 0.08em;
    color: var(--dim);
    margin-bottom: 6px;
  }

  .row {
    display: flex;
    gap: 8px;
    align-items: flex-end;
  }

  textarea {
    flex: 1;
    min-width: 0;
    resize: none;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 13px;
    line-height: 20px;
    padding: 8px 10px;
    outline: none;
  }

  textarea:focus {
    border-color: var(--acc);
  }

  .send {
    width: 40px;
    height: 40px;
    padding: 0;
    font-size: 15px;
    border-color: var(--accln);
    color: var(--acct);
  }

  .send.stop {
    font-size: 11px;
    border-color: var(--acct);
  }

  .err {
    margin-top: 6px;
    font-size: 10px;
    color: var(--acct);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
