<script lang="ts">
  import { store, agentLabel, withoutReportBlock, REPORT_REMINDER, type Run } from "./store.svelte";
  import Markdown from "./Markdown.svelte";
  import ReportCard from "./ReportCard.svelte";
  import StepGroup from "./StepGroup.svelte";
  import { isActive, shape } from "./steps";
  import Working from "./Working.svelte";
  import { t } from "./i18n.svelte";
  import { untrack } from "svelte";
  import { FOLLOWING, followed, reached, resized, scrolled, type Stick } from "./scroll";

  /**
   * A worker worker's session, read like a conversation: what the conductor
   * sent it, then how the worker answered, turn by turn. The worker's turn
   * unfolds live with the same prose and tool lines as the conductor's.
   */
  const runs = $derived(store.trackRuns.filter((r) => r.session === store.openWorker));
  const agent = $derived(runs.at(-1)?.agent ?? "");
  const live = $derived(runs.some((r) => r.status === "running" || r.status === "connecting"));

  // Sticking to the foot, the same way the conversation does it: one
  // observer over the content, every decision in scroll.ts. Watching the
  // height rather than a list of fields is what lets a report card count
  // -- it arrives, and then fills in once its report has been read, and
  // neither step is a change in `runs.length` or `message.length`.
  let scroller = $state<HTMLDivElement>();
  let content = $state<HTMLDivElement>();
  let stick = $state<Stick>({ ...FOLLOWING });

  /** Put the view at the foot, and remember that it was us who did it. */
  function follow() {
    if (!scroller) return;
    const box = scroller;
    box.scrollTop = box.scrollHeight;
    // `untrack`: called from inside effects, and `followed` reads `stick`
    // only to replace it. Tracked, the effect depends on its own write and
    // Svelte fails it with `effect_update_depth_exceeded`. See Timeline.
    stick = untrack(() => followed(stick, box.scrollTop));
  }

  function onScroll() {
    if (scroller) stick = scrolled(stick, scroller);
  }

  // The human reaching back is read from their own input, not inferred from
  // the scroll event it causes: a scroll event cannot say who moved the view,
  // and while a turn streams, guessing from the position loses the gesture.
  // See `reached` in scroll.ts.
  //
  // Listeners rather than handler attributes: they are passive -- nothing here
  // calls preventDefault, and saying so keeps scrolling off the main thread --
  // and a plain scroll box has no ARIA role that would justify the attributes.
  $effect(() => {
    const box = scroller;
    if (!box) return;
    const pull = () => {
      stick = reached(stick, box, true);
    };

    const wheel = (e: WheelEvent) => {
      if (e.deltaY < 0) pull();
    };
    const keys = (e: KeyboardEvent) => {
      if (e.key === "PageUp" || e.key === "ArrowUp" || e.key === "Home") pull();
    };
    let touchY = 0;
    const start = (e: TouchEvent) => {
      touchY = e.touches[0]?.clientY ?? 0;
    };
    // A finger travelling down the screen pulls the conversation back up.
    const move = (e: TouchEvent) => {
      const y = e.touches[0]?.clientY ?? touchY;
      if (y > touchY) pull();
      touchY = y;
    };

    const passive = { passive: true } as const;
    box.addEventListener("wheel", wheel, passive);
    box.addEventListener("keydown", keys);
    box.addEventListener("touchstart", start, passive);
    box.addEventListener("touchmove", move, passive);
    return () => {
      box.removeEventListener("wheel", wheel);
      box.removeEventListener("keydown", keys);
      box.removeEventListener("touchstart", start);
      box.removeEventListener("touchmove", move);
    };
  });

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
    observer.observe(box);
    return () => observer.disconnect();
  });

  // Another worker opens at its newest.
  $effect(() => {
    void store.openWorker;
    stick = { ...FOLLOWING };
    follow();
  });

  function cleanText(text: string): string {
    // The report block shows as a card under the turn.
    return withoutReportBlock(text)
      .replace(/Notice: [\w:.-]+ says: .*?(?=Notice: [\w:.-]+ says: |\n|$)/g, "")
      .replace(/[ \t]+\n/g, "\n")
      .replace(/\n{3,}/g, "\n\n");
  }

  const isLive = (run: Run) => run.status === "running" || run.status === "connecting";

  /** A turn's prose and steps in order, each steps block knowing whether it is still growing. */
  function turn(run: Run) {
    const blocks = shape(run.segments, (text) => !!cleanText(text).trim());
    return blocks.map((block, i) => ({ block, active: isActive(blocks, i, isLive(run)) }));
  }

  /**
   * A turn the human stopped by hand, either way it can end: the agent took
   * the cancel and ended the turn with ACP's `cancelled`, or it did not and
   * its session was killed after the grace. Either way the run stays in the
   * record — stopped, not failed.
   */
  function stopped(run: Run): boolean {
    if (run.status === "done") return (run.stopReason ?? "").toLowerCase() === "cancelled";
    return run.status === "failed" && run.error === "stopped by the human";
  }

  function secs(ms?: number) {
    if (ms === undefined) return "";
    return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
  }
</script>

<main>
  <header>
    <div class="mlab">{t("worker.worker")} / {store.openWorker}</div>
    <div class="row">
      <h1 class="serif">{store.openWorker}</h1>
      <span class="mono meta">{agentLabel(agent)} · {t("worker.turns", { n: runs.length })}</span>
      {#if live}<Working />{/if}
      <span class="grow"></span>
      <button class="btn" onclick={() => (store.view = "track")}>{t("worker.back")}</button>
    </div>
  </header>

  <div class="scroll" bind:this={scroller} onscroll={onScroll}>
    <div class="content" bind:this={content}>
      {#if runs.length === 0}
        <div class="mono empty">{t("worker.empty")}</div>
      {/if}
      {#each runs as run (run.id)}
        {#if run.prompt.startsWith(REPORT_REMINDER)}
          <!-- The app, not the conductor, asked again for the report. -->
          <div class="sys mono">{t("report.reminded")}</div>
        {:else}
          <!-- What the conductor sent this worker: on the right, as the human's words are on the track. -->
          <div class="from">
            <div class="mlab">{t("worker.conductor")}</div>
            <div class="turn bubble">
              <div class="ctext"><Markdown source={run.prompt} /></div>
            </div>
          </div>
        {/if}

        <!-- How the worker answered, as it happened. -->
        <div class="turn to" class:live={run.status === "running" || run.status === "connecting"} class:failed={run.status === "failed"}>
          <div class="head">
            <span class="mlab">{t("worker.worker")}</span>
            <span class="mono meta">{run.id} · {t("worker.tools", { n: run.toolCount })}{run.durationMs !== undefined && run.durationMs !== null ? ` · ${secs(run.durationMs)}` : ""}</span>
            {#if stopped(run)}<span class="mono stopmark">{t("worker.stopped")}</span>{/if}
            {#if run.status === "running" || run.status === "connecting"}<Working />{/if}
          </div>
          {#each turn(run) as { block, active }, i (block.kind === "steps" ? `s${block.at}` : `t${i}`)}
            {#if block.kind === "text"}
              <div class="ctext"><Markdown source={cleanText(block.text)} /></div>
            {:else}
              <StepGroup steps={block.steps} live={isLive(run)} {active} />
            {/if}
          {/each}
          {#if run.segments.length === 0 && run.message.trim()}
            <div class="ctext"><Markdown source={cleanText(run.message)} /></div>
          {/if}
          {#if run.status === "done" || run.status === "failed"}
            <ReportCard run={run.id} compact />
          {/if}
          {#if run.status === "failed" && run.error && !stopped(run)}
            <p class="ctext bad">{run.error}</p>
          {:else if run.segments.length === 0 && !run.message.trim() && (run.status === "connecting" || run.status === "running")}
            <p class="ctext dim">{t("worker.starting")}</p>
          {/if}
        </div>
      {/each}
    </div>
  </div>
</main>

<style>
  .sys {
    margin: 0 0 14px;
    font-size: 10px;
    letter-spacing: 0.12em;
    color: var(--lab);
  }

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

  /* A turn the human stopped: said once, on the turn itself. */
  .stopmark {
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--warn);
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

  .ctext + .ctext {
    margin-top: 8px;
  }









</style>
