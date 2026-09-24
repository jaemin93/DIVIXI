<script lang="ts">
  import { store, agentLabel, type Run, type Segment, type Tool } from "./store.svelte";
  import Markdown from "./Markdown.svelte";
  import Working from "./Working.svelte";
  import { t } from "./i18n.svelte";

  /**
   * A worker lane's session, read like a conversation: what the conductor
   * sent it, then how the worker answered, turn by turn. The worker's turn
   * unfolds live with the same prose and tool lines as the conductor's.
   */
  const runs = $derived(store.trackRuns.filter((r) => r.lane === store.openLane));
  const agent = $derived(runs.at(-1)?.agent ?? "");
  const live = $derived(runs.some((r) => r.status === "running" || r.status === "connecting"));

  let scroller = $state<HTMLDivElement>();
  $effect(() => {
    void runs.length;
    void runs.at(-1)?.message.length;
    void runs.at(-1)?.toolCount;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });

  function cleanText(text: string): string {
    return text
      .replace(/Notice: [\w:.-]+ says: .*?(?=Notice: [\w:.-]+ says: |\n|$)/g, "")
      .replace(/[ \t]+\n/g, "\n")
      .replace(/\n{3,}/g, "\n\n");
  }

  type Shown = Exclude<Segment, { kind: "tool" }> | { kind: "tools"; tools: Tool[] };
  function collapse(segments: Segment[]): Shown[] {
    const out: Shown[] = [];
    for (const seg of segments) {
      const last = out.at(-1);
      if (seg.kind === "tool") {
        if (last && last.kind === "tools") last.tools.push(seg.tool);
        else out.push({ kind: "tools", tools: [seg.tool] });
      } else if (seg.kind === "text" && !cleanText(seg.text).trim()) {
        continue;
      } else {
        out.push(seg);
      }
    }
    return out;
  }

  function toolLabel(title: string): string {
    return title.replace(/^mcp__[a-z0-9_-]+__/i, "").replace(/^mcp\.[a-z0-9_-]+\./i, "");
  }

  function secs(ms?: number) {
    if (ms === undefined) return "";
    return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
  }
</script>

<main>
  <header>
    <div class="mlab">{t("lane.worker")} / {store.openLane}</div>
    <div class="row">
      <h1 class="serif">{store.openLane}</h1>
      <span class="mono meta">{agentLabel(agent)} · {t("lane.turns", { n: runs.length })}</span>
      {#if live}<Working />{/if}
      <span class="grow"></span>
      <button class="btn" onclick={() => (store.view = "track")}>{t("lane.back")}</button>
    </div>
  </header>

  <div class="scroll" bind:this={scroller}>
    {#if runs.length === 0}
      <div class="mono empty">{t("lane.empty")}</div>
    {/if}
    {#each runs as run (run.id)}
      <!-- What the conductor sent this lane: on the right, as the human's words are on the track. -->
      <div class="from">
        <div class="mlab">{t("lane.conductor")}</div>
        <div class="turn bubble">
          <div class="ctext"><Markdown source={run.prompt} /></div>
        </div>
      </div>

      <!-- How the worker answered, as it happened. -->
      <div class="turn to" class:live={run.status === "running" || run.status === "connecting"} class:failed={run.status === "failed"}>
        <div class="head">
          <span class="mlab">{t("lane.worker")}</span>
          <span class="mono meta">{run.id} · {t("lane.tools", { n: run.toolCount })}{run.durationMs !== undefined && run.durationMs !== null ? ` · ${secs(run.durationMs)}` : ""}</span>
          {#if run.status === "running" || run.status === "connecting"}<Working />{/if}
        </div>
        {#each collapse(run.segments) as seg, i (i)}
          {#if seg.kind === "text"}
            <div class="ctext"><Markdown source={cleanText(seg.text)} /></div>
          {:else if seg.kind === "thought"}
            {#if cleanText(seg.text).trim()}<p class="ctext thought">{cleanText(seg.text).trim()}</p>{/if}
          {:else}
            {@const tool = seg.tools[seg.tools.length - 1]}
            {@const running = tool.status !== "completed" && tool.status !== "failed"}
            <div class="toolline mono" class:running>
              <span class="tdot" class:pulse={running}></span>
              <span class="tk">{tool.toolKind}</span>
              <span class="tt">{toolLabel(tool.title)}</span>
              {#if seg.tools.length > 1}<span class="tcount">+{seg.tools.length - 1}</span>{/if}
              <span class="grow"></span>
              <span class="tst" class:bad={tool.status === "failed"}>{tool.status}</span>
            </div>
          {/if}
        {/each}
        {#if run.segments.length === 0 && run.message.trim()}
          <div class="ctext"><Markdown source={cleanText(run.message)} /></div>
        {/if}
        {#if run.status === "failed" && run.error}
          <p class="ctext bad">{run.error}</p>
        {:else if run.segments.length === 0 && !run.message.trim() && (run.status === "connecting" || run.status === "running")}
          <p class="ctext dim">{t("lane.starting")}</p>
        {/if}
      </div>
    {/each}
  </div>
</main>

<style>
  main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background-image: radial-gradient(var(--dot) 1px, transparent 1px);
    background-size: 22px 22px;
  }

  header {
    padding: 26px 34px 16px;
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-top: 8px;
  }

  h1 {
    margin: 0;
    font-weight: 400;
    font-size: 30px;
    line-height: 1.1;
    color: var(--hi);
  }

  .meta {
    font-size: 11px;
    letter-spacing: 0.1em;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }


  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 34px 28px;
  }

  .empty {
    padding: 40px 0;
    font-size: 11px;
    color: var(--lab);
  }

  .turn {
    max-width: 720px;
    margin-bottom: 18px;
    padding: 13px 17px 15px;
    border: 1px solid var(--line);
    background: var(--card);
  }

  .from {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    margin-bottom: 18px;
  }

  .from .mlab {
    margin-bottom: 7px;
  }

  .turn.bubble {
    max-width: 540px;
    margin-bottom: 0;
    border: 1px solid var(--accln);
    border-bottom: 2px solid var(--acc);
    background: var(--accbg);
  }

  .turn.to {
    border-left: 2px solid var(--lines);
  }

  .turn.to.live {
    border-left-color: var(--ok);
  }

  .turn.to.failed {
    border-left-color: var(--warn);
  }

  .turn .mlab {
    margin-bottom: 8px;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .ctext {
    margin: 0;
    font-family: var(--sans);
    font-size: var(--chat-fs);
    line-height: 1.7;
    color: var(--txt);
    white-space: pre-wrap;
  }

  .ctext.dim {
    color: var(--lab);
  }

  .ctext.bad {
    color: var(--acct);
  }

  .ctext.thought {
    color: var(--lab);
    font-style: italic;
    font-size: calc(var(--chat-fs) - 1px);
  }

  .ctext + .ctext,
  .toolline + .ctext,
  .ctext + .toolline {
    margin-top: 8px;
  }

  .toolline {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 22px;
    font-size: 11px;
    color: var(--lab);
  }

  .toolline.running {
    color: var(--dim);
  }

  .tdot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--idle);
    flex-shrink: 0;
  }

  .toolline.running .tdot {
    background: var(--ok);
  }

  .tk {
    letter-spacing: 0.12em;
    text-transform: uppercase;
    font-size: 9px;
  }

  .tt {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--body);
  }

  .tst {
    font-size: 9px;
    letter-spacing: 0.12em;
  }

  .tst.bad {
    color: var(--acct);
  }

  .tcount {
    font-size: 9px;
    letter-spacing: 0.1em;
    color: var(--lab);
    border: 1px solid var(--line);
    padding: 1px 5px;
  }
</style>
