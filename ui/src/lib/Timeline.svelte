<script lang="ts">
  import ReportCard from "./ReportCard.svelte";
  import { store, agentLabel, splitAttachments, splitKnowledge, wasStopped, REPORT_PREFIX, DECISION_PREFIX, PERMISSION_PREFIX, type Decision, type Segment, type Tool } from "./store.svelte";
  import Mark from "./Mark.svelte";
  import DecisionCard from "./DecisionCard.svelte";
  import Working from "./Working.svelte";
  import Markdown from "./Markdown.svelte";
  import { t } from "./i18n.svelte";
  import { untrack } from "svelte";
  import { FOLLOWING, followed, resized, scrolled, type Stick } from "./scroll";

  // ----- sticking to the bottom -----
  //
  // All of the scrolling lives here, in one place, and the decisions live
  // in scroll.ts. The rule is the one people expect of a conversation:
  // while the view is at the bottom it follows whatever arrives, and the
  // moment it is scrolled up it holds still. Nothing else in this file
  // touches scrollTop.
  //
  // What moves the content is watched with a ResizeObserver rather than by
  // listing reactive dependencies: streamed text, a tool line, a decision
  // card, a waiting-report line, the report card that replaces it, an
  // image that finished loading, the track header folding — they all
  // change the same thing, the height of `.content`, and the observer sees
  // all of them. A dependency list only ever sees the ones it was told
  // about, and this conversation grows down several paths that have
  // nothing to do with each other.

  let scroller = $state<HTMLDivElement>();
  let content = $state<HTMLDivElement>();

  /** Where the human is reading; see scroll.ts for what moves it. */
  let stick = $state<Stick>({ ...FOLLOWING });

  /** Put the view at the foot, and remember that it was us who did it. */
  function follow() {
    if (!scroller) return;
    const box = scroller;
    box.scrollTop = box.scrollHeight;
    // Read back rather than assume: the browser clamps, and under page
    // zoom it clamps to a fraction.
    //
    // `untrack` because this is called from inside effects (a conversation
    // opening, a message sent), and `followed` reads `stick` only to replace
    // it wholesale. Tracked, that read makes those effects depend on what
    // they themselves write, and Svelte tears the component down with
    // `effect_update_depth_exceeded`.
    stick = untrack(() => followed(stick, box.scrollTop));
  }

  /**
   * Go to the newest and follow it again. Instantly, on purpose: a smooth
   * glide fires scroll events on the way down that are indistinguishable
   * from the human taking over, and a browser cancels it the moment they
   * touch the wheel. There is nothing to read between here and the foot.
   */
  function toBottom() {
    follow();
  }

  function onScroll() {
    if (scroller) stick = scrolled(stick, scroller);
  }

  // The content growing (or the box around it changing size) is the only
  // cue needed: follow it when stuck, and otherwise remember that something
  // came in below so the button can offer it.
  $effect(() => {
    const inner = content;
    const box = scroller;
    if (!inner || !box) return;
    let tall = inner.offsetHeight;
    const observer = new ResizeObserver(() => {
      const grew = inner.offsetHeight > tall;
      tall = inner.offsetHeight;
      const { next, follow: chase } = resized(stick, grew);
      stick = next;
      if (chase) follow();
    });
    observer.observe(inner);
    // The box itself resizes when the track header folds or the window
    // does: at the bottom, stay at the bottom; scrolled up, stay put.
    observer.observe(box);
    return () => observer.disconnect();
  });

  // Another conversation opens at its newest.
  $effect(() => {
    void store.chatKey;
    stick = { ...FOLLOWING };
    follow();
  });

  /** The human's own additions: a message just sent, or one put in the line. */
  const ownCount = $derived(store.chatQueue.length + store.chatRuns.filter((r) => r.id.startsWith("pending-")).length);
  let ownSeen = -1;
  let ownKey = "";

  // Their own message always brings the view back down to it, wherever they
  // had scrolled to. Nothing the agent says does that.
  $effect(() => {
    const key = store.chatKey;
    const mine = ownCount;
    if (key !== ownKey) {
      ownKey = key;
      ownSeen = mine;
      return;
    }
    if (mine > ownSeen) toBottom();
    ownSeen = mine;
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
   * a terminal does with 
: the latest tool shows, earlier ones become a count.
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
  /** The newest report turn of each worker: its card carries the unmerged changes. */
  const latestReport = $derived.by(() => {
    const out: Record<string, string> = {};
    for (const r of conductorRuns) {
      if (!r.prompt.startsWith(REPORT_PREFIX)) continue;
      const w = /\bworker=(\S+)/.exec(r.prompt.split("\n")[0])?.[1];
      if (w) out[w] = r.id;
    }
    return out;
  });

  // ----- reports the conductor has not taken yet -----
  //
  // A worker can run for an hour, and for a while its report could reach a
  // closed or busy conductor and be dropped on the floor. It is kept now
  // (`meta parked:<track>`), and this is where the human is told: in the
  // conversation, where they are already looking, with the report itself
  // underneath so the wait costs them nothing.

  /** Worker runs whose report is already in this conversation. */
  const deliveredRuns = $derived.by(() => {
    const out = new Set<string>();
    for (const r of conductorRuns) {
      if (!r.prompt.startsWith(REPORT_PREFIX)) continue;
      const wr = /\brun=(\S+)/.exec(r.prompt.split("\n")[0])?.[1];
      if (wr) out.add(wr);
    }
    return out;
  });

  /**
   * What is waiting, minus anything whose report has just landed. Between
   * the core starting the conductor's turn and it clearing the waiting
   * list there is a moment where both are true; without this the report
   * would show twice.
   */
  const waiting = $derived(store.chatParked.filter((w) => !(w.run && deliveredRuns.has(w.run))));

  /** Workers with a report waiting: that one is their newest, not the delivered one. */
  const waitingWorkers = $derived(new Set(waiting.map((w) => w.worker).filter((w): w is string => !!w)));

  /** Nothing is on its way by itself while the conductor is closed. */
  const conductorClosed = $derived(!store.chatArtifact && !!store.track && !store.isActive(store.track));
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

<div class="tl">
  <div class="scroll" bind:this={scroller} onscroll={onScroll}>
    <div class="content" bind:this={content}>
      {#if conductorRuns.length === 0 && store.chatQueue.length === 0}
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
          {@const latest = latestReport[worker] === run.id && !waitingWorkers.has(worker)}
          {#if workerRun && store.reports[workerRun]}
            <ReportCard run={workerRun} {worker} track={run.track} {latest} />
          {:else}
            <div class="sys mono">{t("timeline.reportArrived")} · {head.replace(REPORT_PREFIX, "").trim()}</div>
            {#if workerRun}<ReportCard run={workerRun} {worker} track={run.track} {latest} />{/if}
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
            {:else if wasStopped(run)}
              <!-- The human stopped this turn: what it had said stays. -->
              <span class="mono stopped">{t("timeline.stopped")}</span>
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

      <!-- Sent while a turn was in flight: in the conversation already, waiting
           its turn. Each can be taken back until it goes. One marked "unsent"
           came back with the app and waits on the human, not on the turn. -->
      {#each store.chatQueue as q (q.id)}
        <div class="me">
          <div class="mlab">{t("timeline.me")}</div>
          <div class="bubble waiting" class:held={q.held}>
            <div class="qhead">
              <span class="mono qbadge" class:heldbadge={q.held}>{q.held ? t("timeline.unsent") : t("timeline.queued")}</span>
              <span class="grow"></span>
              {#if q.held}
                <button class="qsend mono" type="button" onclick={() => store.releaseQueued(q.id)} title={t("timeline.sendNowTitle")}>{t("timeline.sendNow")}</button>
              {/if}
              <button class="qdrop" type="button" onclick={() => store.cancelQueued(q.id)} title={t("timeline.unqueue")} aria-label={t("timeline.unqueue")}>✕</button>
            </div>
            {#if q.text.trim()}<p>{q.text}</p>{/if}
            {#if q.picks.length}
              <div class="files" aria-label={t("composer.kb.attached")}>
                <!-- By position: the list never reorders, and an index
                     cannot collide the way a saved id can. -->
                {#each q.picks as p, i (i)}
                  <span class="file kbchip" title={p.title}><span aria-hidden="true">📚</span><span>{p.title}</span></span>
                {/each}
              </div>
            {/if}
            {#if q.files.length}
              <div class="files" class:alone={!q.text.trim()} aria-label={t("timeline.attached")}>
                {#each q.files as f (f)}
                  <span class="file" title={f}>
                    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2" aria-hidden="true"><path d="M4 1.5h5l3 3v10H4z" /><path d="M9 1.5v3h3" /></svg>
                    <span>{f.split(/[\\/]/).pop()}</span>
                  </span>
                {/each}
              </div>
            {/if}
          </div>
        </div>
      {/each}
      <!-- Always in the tree, empty or not: a screen reader announces what is
           put *into* a live region, and one that appears with its first
           message is easy to miss. Polite, and only additions — nothing has
           failed here, and a report going in is not worth interrupting for.
           Same shape as the error stack (ErrorToasts.svelte). -->
      <div class="parked" class:empty={waiting.length === 0} role="status" aria-live="polite" aria-atomic="false" aria-relevant="additions">
        <!-- Keyed on the core's id. `what` is a human sentence and not
           unique by construction, and Svelte throws on a duplicate key
           in release builds too, taking the conversation with it. -->
      {#each waiting as w (w.id)}
          <div class="sys mono pline">
            <span class="dot pulse" aria-hidden="true"></span>
            <span title={t("timeline.parkedNote")}>
              {w.worker ? t("timeline.parked", { worker: w.worker }) : t("timeline.parkedOther", { what: w.what })}
            </span>
            <span class="grow"></span>
            {#if conductorClosed}
              <button class="psend mono" type="button" onclick={() => store.openConductor(store.track)} title={t("timeline.parkedDeliverTitle")}>
                {t("timeline.parkedDeliver")}
              </button>
            {/if}
          </div>
          <!-- The report itself, read now rather than when the conductor gets
               round to it. Out of the live region: the line above says what
               happened, and reading a whole card aloud on arrival would not
               help anyone. -->
          {#if w.run}
            <div aria-live="off">
              <ReportCard run={w.run} worker={w.worker ?? ""} track={w.track} latest />
            </div>
          {/if}
        {/each}
      </div>

      {#if !store.chatArtifact && store.handoffs[store.track]}
        {@const h = store.handoffs[store.track]}
        <div class="sys mono"><span class="dot pulse"></span> {t("timeline.handoff", { from: agentLabel(h.from), to: agentLabel(h.to) })}</div>
      {/if}
    </div>
  </div>

  {#if stick.missed}
    <!-- Reading back while the agent answered: the way down, and back to
         following it. -->
    <button class="jump mono" type="button" onclick={() => toBottom()}>
      <span class="jarrow" aria-hidden="true">↓</span>{t("timeline.toBottom")}
    </button>
  {/if}
</div>

<style>
  .tl {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 34px 28px;
  }

  /* The "jump to newest" button floats over the foot of the conversation,
     clear of the composer below it. */
  .jump {
    position: absolute;
    right: 34px;
    bottom: 14px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 12px;
    background: var(--card);
    border: 1px solid var(--acc);
    color: var(--hi);
    font-size: 10px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    cursor: pointer;
    box-shadow: 0 2px 10px rgb(0 0 0 / 0.28);
  }

  .jump:hover {
    background: var(--sel);
  }

  .jarrow {
    font-size: 12px;
    color: var(--acc);
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
  /* The waiting block adds nothing of its own when there is nothing in it:
     it is here for the screen reader, not for the layout. */
  .parked.empty {
    display: none;
  }

  .pline {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .pline .dot {
    width: 6px;
    height: 6px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--warn);
  }

  .psend {
    padding: 2px 8px;
    background: transparent;
    border: 1px solid var(--lineq);
    color: var(--lab);
    font-size: 10px;
  }

  .psend:hover {
    color: var(--hi);
    border-color: var(--acc);
  }

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

  /* A message waiting its turn: the same bubble, held back. */
  .bubble.waiting {
    border-style: dashed;
    border-bottom-width: 1px;
    background: transparent;
    opacity: 0.82;
  }

  .qhead {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }

  .qbadge {
    font-size: 9px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--lab);
    border: 1px solid var(--line);
    padding: 1px 6px;
  }

  /* Came back with the app: it is the human's to send, so it says so more
     plainly than one merely waiting for a turn. */
  .bubble.waiting.held {
    border-color: var(--warn);
    opacity: 1;
  }

  .qbadge.heldbadge {
    color: var(--warn);
    border-color: var(--warn);
  }

  .qsend {
    height: 18px;
    padding: 0 8px;
    background: transparent;
    border: 1px solid var(--accln);
    color: var(--acct);
    font-size: 9px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    cursor: pointer;
  }

  .qsend:hover {
    border-color: var(--acc);
    background: var(--sel);
  }

  .qdrop {
    width: 18px;
    height: 18px;
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--lab);
    font-size: 11px;
    line-height: 1;
    cursor: pointer;
  }

  .qdrop:hover {
    color: var(--acct);
  }

  /* A turn the human stopped; what it had said is kept. */
  .stopped {
    font-size: 9px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--acct);
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
