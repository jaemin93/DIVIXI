<script lang="ts">
  import ReportCard from "./ReportCard.svelte";
  import { store, agentLabel, splitAttachments, splitKnowledge, REPORT_PREFIX, DECISION_PREFIX, PERMISSION_PREFIX, type Decision, type Segment, type Tool } from "./store.svelte";
  import Mark from "./Mark.svelte";
  import DecisionCard from "./DecisionCard.svelte";
  import Working from "./Working.svelte";
  import Markdown from "./Markdown.svelte";
  import { t } from "./i18n.svelte";

  let scroller = $state<HTMLDivElement>();

  // Follow the tail while a run is live.
  $effect(() => {
    void store.chatRuns.length;
    void store.activeRun?.message.length;
    void store.activeRun?.toolCount;
    void store.chatDecisions.length;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });

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

  /** Decisions sit under the conductor turn that asked; the rest (asked
   *  by a run no longer here) go at the end. */
  /** The human and the conductor, or the human and an artifact's agent. */
  const conductorRuns = $derived(store.chatRuns);
  const decisionsByRun = $derived.by(() => {
    const byRun: Record<string, Decision[]> = {};
    const orphans: Decision[] = [];
    const ids = new Set(conductorRuns.map((r) => r.id));
    for (const d of store.chatDecisions) {
      if (d.run && ids.has(d.run)) (byRun[d.run] ??= []).push(d);
      else orphans.push(d);
    }
    return { byRun, orphans };
  });

  /** "fix-parser · Write src/a.ts" from a worker's permission question. */
  function permissionLine(prompt: string): string {
    const [head, second = ""] = prompt.split("\n");
    const worker = head.match(/worker=(\S+)/)?.[1] ?? "";
    return `${worker} · ${second.replace(/^wants to: /, "")}`;
  }

  /** "#3 · B. label" from the turn that carried an answer to the conductor. */
  function decisionLine(prompt: string): string {
    const lines = prompt.split("\n");
    const id = lines[0].replace(DECISION_PREFIX, "").trim();
    const answer = lines.find((l) => l.startsWith("answer: "))?.slice(8) ?? "";
    return answer ? `${id} · ${answer}` : id;
  }

  /** Tool titles from MCP arrive as `mcp__divixi__spawn_worker`; show the tool. */
  function toolLabel(title: string): string {
    return title.replace(/^mcp__[a-z0-9_-]+__/i, "").replace(/^mcp\.[a-z0-9_-]+\./i, "");
  }

</script>

<div class="scroll" bind:this={scroller}>
  {#if conductorRuns.length === 0}
    <div class="empty">
      <div class="emptymark"><Mark size={56} /></div>
      <div class="mlab">{t("timeline.empty")}</div>
      <p class="serif">{store.chatArtifact ? t("design.chatEmpty") : t("timeline.emptyHint")}</p>
    </div>
  {/if}

  <!-- The track is the human and the conductor. Worker work is the conductor's
       to relay; workers themselves are read in their own view. -->
  {#each conductorRuns as run (run.id)}
    {#if run.prompt.startsWith(REPORT_PREFIX)}
      {@const head = run.prompt.split("\n")[0]}
      {@const workerRun = /\brun=(\S+)/.exec(head)?.[1] ?? ""}
      {@const worker = /\bworker=(\S+)/.exec(head)?.[1] ?? ""}
      {#if workerRun && store.reports[workerRun]}
        <ReportCard run={workerRun} {worker} />
      {:else}
        <div class="sys mono">{t("timeline.reportArrived")} · {head.replace(REPORT_PREFIX, "").trim()}</div>
        {#if workerRun}<ReportCard run={workerRun} {worker} />{/if}
      {/if}
    {:else if run.prompt.startsWith(PERMISSION_PREFIX)}
      <!-- A worker asked; the conductor answers for the human. -->
      <div class="sys mono decided">{t("timeline.permissionAsked")} · {permissionLine(run.prompt)}</div>
    {:else if run.prompt.startsWith(DECISION_PREFIX)}
      <div class="sys mono decided">{t("timeline.decisionSent")} · {decisionLine(run.prompt)}</div>
    {:else}
      {@const attached = splitAttachments(run.prompt)}
      {@const kb = splitKnowledge(attached.text)}
      {@const said = { text: kb.text, files: attached.files }}
      <div class="me">
        <div class="mlab">{t("timeline.me")}</div>
        <div class="bubble">
          {#if said.text.trim()}<p>{said.text}</p>{/if}
          {#if kb.titles.length}
            <div class="files" class:alone={!said.text.trim()} aria-label={t("composer.kb.attached")}>
              {#each kb.titles as title, i (i)}
                <span class="file kbchip" title={title}><span aria-hidden="true">📚</span><span>{title}</span></span>
              {/each}
            </div>
          {/if}
          {#if said.files.length}
            <div class="files" class:alone={!said.text.trim()} aria-label={t("timeline.attached")}>
              {#each said.files as f (f)}
                {@const rel = store.relativeToTrack(f)}
                <button class="file" type="button" title={f} disabled={!rel} onclick={() => rel && store.openFile(rel)}>
                  <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" aria-hidden="true"><path d="M4 1.5h5l3 3v10H4z" /><path d="M9 1.5v3h3" /></svg>
                  <span>{f.split(/[\\/]/).pop()}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      </div>
    {/if}
    <div class="conductor" class:live={run.status === "connecting" || run.status === "running"}>
      <div class="chead">
        <span class="mlab">{store.chatArtifact ? t("design.agentLabel") : t("timeline.conductor")}</span>
        <span class="mono meta">{agentLabel(run.agent)}</span>
        {#if run.status === "connecting" || run.status === "running"}
          <Working />
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
    {#each decisionsByRun.byRun[run.id] ?? [] as d (d.id)}
      <DecisionCard decision={d} />
    {/each}
  {/each}
  {#each decisionsByRun.orphans as d (d.id)}
    <DecisionCard decision={d} />
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

  /* A worker report arriving for the conductor: a quiet system line. */
  .sys {
    margin: 0 0 12px;
    font-size: 10px;
    letter-spacing: 0.12em;
    color: var(--lab);
  }

  .sys.decided {
    color: var(--warn);
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

  .files {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 10px;
  }

  .kbchip {
    border-color: var(--lines);
  }

  .files.alone {
    margin-top: 0;
  }

  .file {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    max-width: 260px;
    padding: 0 9px;
    background: var(--card);
    border: 1px solid var(--accln);
    color: var(--txt);
    font-size: 12px;
  }

  .file span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file:hover:not(:disabled) {
    color: var(--hi);
    border-color: var(--acc);
  }

  .file:disabled {
    opacity: 1;
    cursor: default;
  }

  .bubble p {
    margin: 0;
    font-family: var(--sans);
    font-size: var(--chat-fs);
    line-height: var(--chat-lh);
    color: var(--txt);
    white-space: pre-wrap;
    word-break: break-word;
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


  .meta {
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .ctext.bad {
    color: var(--acct);
  }

</style>
