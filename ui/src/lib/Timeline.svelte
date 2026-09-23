<script lang="ts">
  import { store, agentLabel, REPORT_PREFIX, type Run, type Segment, type Tool } from "./store.svelte";
  import Mark from "./Mark.svelte";
  import Markdown from "./Markdown.svelte";
  import { t } from "./i18n.svelte";

  let scroller = $state<HTMLDivElement>();

  // Follow the tail while a run is live.
  $effect(() => {
    void store.trackRuns.length;
    void store.activeRun?.message.length;
    void store.activeRun?.toolCount;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });

  function secs(ms?: number) {
    if (ms == null) return "";
    return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
  }

  /** The one line a Running card is allowed to show from below the membrane. */
  function tail(run: Run) {
    const last = run.tools.at(-1);
    if (last) return `${last.toolKind} ${last.title}`;
    const text = run.message || run.thought;
    return text ? text.slice(-90).replace(/\s+/g, " ") : t("timeline.waiting");
  }

  /** Hook chatter (devterm memory and the like) arrives as message text; hide it. */
  function cleanText(text: string): string {
    // "Notice: <Hook> says: …" up to the next notice or line end; they
    // often arrive glued together on one line.
    return text
      .replace(/Notice: [\w:.-]+ says: .*?(?=Notice: [\w:.-]+ says: |\n|$)/g, "")
      .replace(/[ \t]+\n/g, "\n")
      .replace(/\n{3,}/g, "\n\n");
  }

  /**
   * Consecutive tool calls fold into one line that overwrites itself, the way
   * a terminal does with : the latest tool shows, earlier ones become a count.
   */
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

  /** Tool titles from MCP arrive as `mcp__orchestra__spawn_lane`; show the tool. */
  function toolLabel(title: string): string {
    return title.replace(/^mcp__[a-z0-9_-]+__/i, "").replace(/^mcp\.[a-z0-9_-]+\./i, "");
  }

  /** Reports are summaries; the full text lives in the lane. */
  function summary(run: Run): string {
    const text = cleanText(run.message).trim();
    if (!text) return t("timeline.noText");
    return text.length > 600 ? `${text.slice(0, 600)}…` : text;
  }
</script>

<div class="scroll" bind:this={scroller}>
  {#if store.trackRuns.length === 0}
    <div class="empty">
      <div class="emptymark"><Mark size={56} ink="var(--lines)" /></div>
      <div class="mlab">{t("timeline.empty")}</div>
      <p class="serif">{t("timeline.emptyHint")}</p>
    </div>
  {/if}

  <!-- The track is the human and the conductor. Lane work is the conductor's
       to relay; lanes themselves are read in their own view. -->
  {#each store.trackRuns.filter((r) => r.lane === "conductor") as run (run.id)}
    {#if run.lane === "conductor"}
      {#if run.prompt.startsWith(REPORT_PREFIX)}
        <div class="sys mono">{t("timeline.reportArrived")} · {run.prompt.split("\n")[0].replace(REPORT_PREFIX, "").trim()}</div>
      {:else}
        <div class="me">
          <div class="mlab">{t("timeline.me")}</div>
          <div class="bubble"><p>{run.prompt}</p></div>
        </div>
      {/if}
      <div class="conductor" class:live={run.status === "connecting" || run.status === "running"}>
        <div class="chead">
          <span class="mlab">{t("timeline.conductor")}</span>
          <span class="mono meta">{agentLabel(run.agent)}</span>
          {#if run.status === "connecting" || run.status === "running"}
            <span class="dot pulse"></span>
          {/if}
        </div>
        <!-- The turn as it unfolds: prose and tool lines in order, like a native session. -->
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
          <!-- Restored from the store: only the folded text survives. -->
          <div class="ctext"><Markdown source={cleanText(run.message)} /></div>
        {/if}
        {#if run.status === "failed" && run.error}
          <p class="ctext bad">{run.error}</p>
        {:else if run.segments.length === 0 && !run.message.trim() && (run.status === "connecting" || run.status === "running")}
          <p class="ctext dim">{t("timeline.thinking")}</p>
        {/if}
      </div>
    {:else}

    {#if run.status === "connecting" || run.status === "running"}
      <div class="card running">
        <div class="head">
          <span class="dot pulse"></span>
          <span class="mono tag ok">{run.status === "connecting" ? "CONNECTING" : "RUNNING"}</span>
          <span class="mono meta">lane {run.lane} · {run.id} · {agentLabel(run.agent)}</span>
          <span class="grow"></span>
          <span class="mono meta">{run.toolCount} tools</span>
        </div>
        <div class="mono tailline">› {tail(run)}</div>
      </div>
    {:else}
      <div class="card report" class:failed={run.status === "failed"}>
        <button class="rowhead" onclick={() => store.inspect(run.id)}>
          <span class="mono chev">{store.inspecting === run.id ? "▾" : "▸"}</span>
          <span class="col"><span class="mlab-sm">LANE</span><span class="mono val">{run.lane}</span></span>
          <span class="col"><span class="mlab-sm">RUN</span><span class="mono val">{run.id}</span></span>
          <span class="col"><span class="mlab-sm">AGENT</span><span class="mono val">{agentLabel(run.agent)}</span></span>
          <span class="col">
            <span class="mlab-sm">RESULT</span>
            <span class="mono val" class:ok={run.status === "done"} class:bad={run.status === "failed"}>
              • {run.status === "done" ? t("timeline.done") : t("timeline.failed")}
            </span>
          </span>
          <span class="col"><span class="mlab-sm">TOOLS</span><span class="mono val">{run.toolCount}</span></span>
          <span class="col"><span class="mlab-sm">DURATION</span><span class="mono val">{secs(run.durationMs)}</span></span>
          <span class="grow"></span>
          <span class="mlab">REPORT</span>
        </button>

        {#if store.inspecting === run.id}
          <div class="reportbody">
            <div class="field">
              <div class="mlab-sm k">{t("timeline.didWhat")}</div>
              <div class="v"><Markdown source={summary(run)} /></div>
            </div>
            {#if run.plan.length}
              <div class="field">
                <div class="mlab-sm k">{t("timeline.plan")}</div>
                <div class="v">
                  {#each run.plan as entry}<div>→ {entry}</div>{/each}
                </div>
              </div>
            {/if}
            {#if run.error}
              <div class="field">
                <div class="mlab-sm k">{t("timeline.failure")}</div>
                <div class="v bad">{run.error}</div>
              </div>
            {/if}
            <div class="membrane">
              <span class="mlab-sm acc">{t("timeline.membrane")}</span>
              <span class="note">{t("timeline.membraneNote")}</span>
            </div>
          </div>
        {/if}
      </div>
    {/if}
    {/if}
  {/each}
</div>

<style>
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 34px 28px;
  }

  .empty {
    padding: 60px 0;
    text-align: center;
  }

  .empty p {
    margin: 10px 0 0;
    font-size: 19px;
    color: var(--dim);
  }

  .emptymark {
    display: flex;
    justify-content: center;
    margin-bottom: 18px;
  }

  /* A lane report arriving for the conductor: a quiet system line. */
  .sys {
    margin: 0 0 12px;
    font-size: 10px;
    letter-spacing: 0.12em;
    color: var(--lab);
  }

  .me {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    margin-bottom: 22px;
  }

  .me .mlab {
    margin-bottom: 7px;
  }

  .bubble {
    max-width: 540px;
    border: 1px solid var(--accln);
    border-bottom: 2px solid var(--acc);
    background: var(--accbg);
    padding: 13px 16px;
  }

  .bubble p {
    margin: 0;
    font-family: var(--sans);
    font-size: var(--chat-fs);
    line-height: var(--chat-lh);
    color: var(--txt);
  }

  /* The conductor's turn: the same surface as the other cards, with the
     accent on the left, but its body is prose, not a report. */
  .conductor {
    max-width: 680px;
    margin-bottom: 22px;
    padding: 13px 17px 15px;
    border: 1px solid var(--line);
    border-left: 2px solid var(--lines);
    background: var(--card);
  }

  .conductor.live {
    border-left-color: var(--ok);
  }

  .chead {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 10px;
  }

  .chead .dot {
    background: var(--ok);
  }

  /* Conversation text follows the interface font and the conversation size. */
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

  /* Thinking: quieter than speech. */
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

  /* One line per tool call, as a native session prints them. */
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

  .ctext.bad {
    color: var(--acct);
  }

  .card {
    margin-bottom: 20px;
    border: 1px solid var(--line);
    background: var(--card);
  }

  .running {
    border-left: 2px solid var(--okln);
    padding: 14px 17px;
  }

  .report {
    border-left: 2px solid var(--acc);
  }

  .report.failed {
    border-left-color: var(--warn);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 13px;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
  }

  .tag {
    font-size: 10px;
    letter-spacing: 0.18em;
  }

  .tag.ok {
    color: var(--ok);
  }

  .meta {
    font-size: 11px;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .tailline {
    margin-top: 11px;
    font-size: 11px;
    color: var(--lab);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .rowhead {
    width: 100%;
    height: 56px;
    display: flex;
    align-items: center;
    gap: 24px;
    padding: 0 17px;
    background: transparent;
    border: 0;
    text-align: left;
  }

  .chev {
    font-size: 11px;
    color: var(--dim);
    width: 10px;
  }

  .col {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 62px;
  }

  .val {
    font-size: 12px;
    color: var(--txt);
  }

  .val.ok {
    color: var(--ok);
  }

  .val.bad {
    color: var(--acct);
  }

  .reportbody {
    border-top: 1px solid var(--line);
    padding: 18px 19px 16px;
    display: flex;
    flex-direction: column;
    gap: 15px;
  }

  .field {
    display: flex;
    gap: 20px;
  }

  .k {
    width: 66px;
    flex-shrink: 0;
    padding-top: 3px;
  }

  .v {
    font-size: calc(var(--chat-fs) - 2px);
    line-height: 1.75;
    color: var(--body);
  }

  .v.bad {
    color: var(--acct);
  }

  .membrane {
    border-top: 1px dashed var(--lines);
    padding-top: 13px;
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .acc {
    color: var(--acct);
  }

  .note {
    font-size: 12px;
    color: var(--lab);
  }
</style>
