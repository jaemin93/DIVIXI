<script lang="ts">
  import { untrack } from "svelte";
  import { store, type Watched } from "./store.svelte";
  import Popover from "./Popover.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";
  import { whenFull } from "./time";
  import { isCondition, lateness, message, nextLabel, rule, upcoming } from "./scheduleList";
  import { forPopover, isOpen, jobState, lastMoved, watches, when } from "./jobList";

  /**
   * The track's schedules and background jobs, one row each, beside the
   * panel toggle at the top right: what will wake the conductor and when,
   * soonest first, then what the app is watching for it (running ones
   * first, then the ones that ended in the last day). Shown only while the
   * track has one or the other. Other tracks' follow under their own
   * heading, read when the popover opens; the `schedules` and `jobs` events
   * keep every list live after that.
   *
   * A row brings its line in the conversation on screen, and that line's
   * [Show] opens this on its row, expanded (`store.showSchedule`). Cancel
   * is here only. Jobs the same way (`store.showJob`), and they are stopped
   * being watched here only. A running job's row brings its line on screen;
   * one that ended, or another track's, opens the worker that registered it.
   */
  let open = $state(false);
  let expanded = $state<number[]>([]);
  /** Expanded job rows: their own ids, which may be a schedule's too. */
  let jobsExpanded = $state<number[]>([]);

  const narrowQuery = typeof matchMedia === "function" ? matchMedia("(max-width: 640px)") : null;
  let narrow = $state(narrowQuery?.matches ?? false);
  $effect(() => {
    if (!narrowQuery) return;
    const on = () => (narrow = narrowQuery.matches);
    narrowQuery.addEventListener("change", on);
    return () => narrowQuery.removeEventListener("change", on);
  });

  const here = $derived(upcoming(store.schedules.filter((s) => s.track === store.track)));
  const hereJobs = $derived(forPopover(store.jobs.filter((j) => j.track === store.track), store.now));
  /** What the count on the button says: what is still to come or still running. */
  const live = $derived(here.length + hereJobs.filter(isOpen).length);
  const others = $derived.by(() => {
    const out: { track: string; name: string; items: Watched[]; jobs: Watched[] }[] = [];
    for (const tr of store.tracks) {
      if (tr.id === store.track) continue;
      const items = upcoming(store.schedules.filter((s) => s.track === tr.id));
      const jobs = forPopover(store.jobs.filter((j) => j.track === tr.id), store.now);
      if (items.length || jobs.length) out.push({ track: tr.id, name: tr.name, items, jobs });
    }
    return out;
  });
  const locale = $derived(store.lang === "ko" ? "ko-KR" : "en-US");
  const tr = (key: string, vars?: Record<string, string | number>) => t(key as Parameters<typeof t>[0], vars);

  function toggle() {
    open = !open;
    if (open) void store.loadAllWatched();
  }

  function expand(id: number) {
    expanded = expanded.includes(id) ? expanded.filter((x) => x !== id) : [...expanded, id];
  }

  function expandJob(id: number) {
    jobsExpanded = jobsExpanded.includes(id) ? jobsExpanded.filter((x) => x !== id) : [...jobsExpanded, id];
  }

  /** A job of this track that the conversation has a line for: only open ones have one. */
  function hasLine(j: Watched): boolean {
    return isOpen(j) && j.track === store.track;
  }

  /** Whether a job's row leads anywhere: to its line, to the worker that registered it, or to the track of a conductor's own on another track. */
  function jobGoes(j: Watched): boolean {
    return hasLine(j) || !!j.worker || j.track !== store.track;
  }

  async function openJob(j: Watched) {
    if (hasLine(j)) {
      reveal(`job-${j.id}`);
      return;
    }
    open = false;
    if (j.track !== store.track) await store.selectTrack(j.track);
    if (j.worker) store.openWorkerView(j.worker);
  }

  /** A time as a row shows it: `21:40` today, else `10-19 21:40`. */
  const at = (ms: number) => nextLabel(ms, store.now, tr);

  /** A line in the conversation (`schedule-8`, `job-3`), on screen and lit for a moment. A track just opened draws its timeline over the next frames. */
  function reveal(id: string, left = 20) {
    const el = document.getElementById(id);
    if (!el) {
      if (left > 0) setTimeout(() => reveal(id, left - 1), 100);
      return;
    }
    el.scrollIntoView({ block: "center", behavior: "smooth" });
    el.animate([{ backgroundColor: "var(--sel)" }, { backgroundColor: "transparent" }], { duration: 1400, easing: "ease-out" });
  }

  // Asked from the conversation: opened on that row, expanded, in view.
  $effect(() => {
    const focus = store.watchFocus;
    if (!focus) return;
    untrack(() => {
      if (focus.kind === "job") {
        if (!jobsExpanded.includes(focus.id)) jobsExpanded = [...jobsExpanded, focus.id];
      } else if (!expanded.includes(focus.id)) expanded = [...expanded, focus.id];
      if (!open) {
        open = true;
        void store.loadAllWatched();
      }
      bringRow(`${focus.kind}-row-${focus.id}`);
    });
  });

  /** A row scrolled to the middle of the panel and lit for a moment, once the panel has its height. */
  function bringRow(id: string, left = 20) {
    const row = document.getElementById(id);
    const panel = row?.closest<HTMLElement>("[role=dialog]");
    if (!row || !panel || !panel.style.maxHeight) {
      if (left > 0) setTimeout(() => bringRow(id, left - 1), 50);
      return;
    }
    panel.scrollTop = Math.max(0, row.offsetTop - (panel.clientHeight - row.offsetHeight) / 2);
    row.animate([{ backgroundColor: "var(--sel)" }, { backgroundColor: "transparent" }], { duration: 1400, easing: "ease-out" });
  }

  function go(s: Watched) {
    if (s.track === store.track) {
      reveal(`schedule-${s.id}`);
      return;
    }
    open = false;
    void store.selectTrack(s.track).then(() => reveal(`schedule-${s.id}`));
  }
</script>

{#if here.length || hereJobs.length}
  <Popover bind:open align="right" width={narrow ? 10_000 : 380} label={t("schedules.titleBoth")}>
    {#snippet trigger()}
      <button class="tog" class:on={open} type="button" title={t("schedules.titleBoth")} aria-label={t("schedules.titleBoth")} aria-haspopup="dialog" aria-expanded={open} onclick={toggle}>
        <Icon name="clock" size={14} />
        {#if live}<span class="n mono" aria-hidden="true">{live}</span>{/if}
      </button>
    {/snippet}

    <div class="head">
      <span class="title">{t("schedules.titleBoth")}</span>
    </div>
    {#if others.length}<div class="mlab-sm group">{t("schedules.here")}</div>{/if}
    {@render block(here, hereJobs, false)}
    {#if others.length}
      <div class="mlab-sm group">{t("schedules.others")}</div>
      {#each others as g (g.track)}
        <div class="tname" title={g.name}>{g.name}</div>
        {@render block(g.items, g.jobs, true)}
      {/each}
    {/if}
  </Popover>
{/if}

<!-- One track's schedules, then its jobs, each kind named when it has rows. -->
{#snippet block(items: Watched[], jobs: Watched[], other: boolean)}
  {#if items.length}
    <div class="kind" class:sub={other}>{t("schedules.title")}</div>
    {#each items as s (s.id)}
      {@render item(s)}
    {/each}
  {/if}
  {#if jobs.length}
    <div class="kind" class:sub={other}>{t("jobs.title")}</div>
    {#each jobs as j (j.id)}
      {@render jobItem(j)}
    {/each}
  {/if}
{/snippet}

{#snippet jobItem(j: Watched)}
  {@const st = jobState(j)}
  {@const since = when(j)}
  {@const moved = lastMoved(j)}
  {@const full = jobsExpanded.includes(j.id)}
  {@const goes = jobGoes(j)}
  {@const details = [j.what, j.pid ? `pid ${j.pid}` : "", j.log ? tr("jobs.log", { path: j.log }) : "", j.done_pattern ? tr("jobs.donePattern", { p: j.done_pattern }) : ""].filter(Boolean).join("\n")}
  <div class="row" id="job-row-{j.id}">
    <button class="main" type="button" disabled={!goes} onclick={() => openJob(j)} title={!goes ? undefined : hasLine(j) ? t("schedules.show") : j.worker ? t("jobs.openWorker") : t("schedules.openTrack")}>
      <span class="top mono">
        <span class="id">#{j.id}</span>
        <span class="who">{j.worker || t("jobs.conductor")}</span>
        <span class="state {st}">{tr(`jobs.${st}`)}</span>
        {#if since != null}<span title={whenFull(since, store.lang)}>{isOpen(j) ? tr("jobs.since", { time: at(since) }) : at(since)}</span>{/if}
        {#if moved != null}<span title={t("jobs.movedTitle")}>· {tr("jobs.moved", { time: at(moved) })}</span>{/if}
      </span>
      {#if full}
        <span class="msg full">{details}</span>
        {#if j.last_line}<span class="note mono full">{tr("jobs.lastLine", { line: j.last_line })}</span>{/if}
      {:else}
        <span class="msg one" title={details}>{watches(j)}</span>
        <!-- An ending's note quotes the line that finished it: not said twice. -->
        {#if j.last_line && !j.note?.includes(j.last_line)}<span class="note mono one" title={j.last_line}>{j.last_line}</span>{/if}
      {/if}
      <!-- The core keeps a stall's note when the log moves again: a running job's would be stale. -->
      {#if j.note && st !== "running"}<span class="note mono" class:warn={st === "died" || st === "stalled"}>{j.note}</span>{/if}
    </button>
    <div class="side">
      {#if isOpen(j)}
        <button class="cancel mono" type="button" onclick={() => store.unwatchJob(j.id)} title={t("timeline.jobNote")}>{t("timeline.unwatchJob")}</button>
      {/if}
      <button class="more" type="button" aria-expanded={full} onclick={() => expandJob(j.id)} title={full ? t("schedules.less") : t("schedules.more")} aria-label={full ? t("schedules.less") : t("schedules.more")}>
        {full ? "▴" : "▾"}
      </button>
    </div>
  </div>
{/snippet}

{#snippet item(s: Watched)}
  {@const late = lateness(s, store.now)}
  {@const r = rule(s, tr, locale)}
  {@const text = message(s)}
  {@const full = expanded.includes(s.id)}
  {@const long = text.length > 90 || text.includes("\n")}
  <div class="row" id="schedule-row-{s.id}">
    <button class="main" type="button" onclick={() => go(s)} title={s.track === store.track ? t("schedules.show") : t("schedules.openTrack")}>
      <span class="top mono">
        <span class="id">#{s.id}</span>
        {#if s.next_at != null}
          <span class="when" class:late={late === "overdue"} title={whenFull(s.next_at, store.lang)}>{nextLabel(s.next_at, store.now, tr)}</span>
          {#if r}<span class="rule">· {r}</span>{/if}
        {:else if isCondition(s) && r}
          <span class="when cond">{r}</span>
        {/if}
        <!-- A firing that came late says so in its note, below; the tag is for one still waiting. -->
        {#if late === "overdue"}<span class="tag">{t("schedules.late")}</span>{/if}
      </span>
      <span class="msg" class:full title={full ? undefined : text}>{text}</span>
      {#if s.note}<span class="note mono" class:warn={late === "fired-late"}>{s.note}</span>{/if}
    </button>
    <div class="side">
      <button class="cancel mono" type="button" onclick={() => store.cancelSchedule(s.id)} title={t("timeline.cancelSchedule")}>{t("timeline.cancelSchedule")}</button>
      {#if long}
        <button class="more" type="button" aria-expanded={full} onclick={() => expand(s.id)} title={full ? t("schedules.less") : t("schedules.more")} aria-label={full ? t("schedules.less") : t("schedules.more")}>
          {full ? "▴" : "▾"}
        </button>
      {/if}
    </div>
  </div>
{/snippet}

<style>
  /* The panel toggle's square, widened by its count. */
  .tog {
    min-width: 28px;
    height: 28px;
    padding: 0 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--dim);
    flex-shrink: 0;
  }

  .tog:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .tog.on {
    color: var(--hi);
    border-color: var(--acc);
  }

  .n {
    font-size: 10px;
    line-height: 1;
  }

  /* The panel is a flex column that scrolls: what is in it keeps its height. */
  .head,
  .group,
  .tname,
  .row {
    flex-shrink: 0;
  }

  .head {
    display: flex;
    align-items: center;
    padding: 12px 16px 8px;
  }

  .title {
    font-size: 14px;
    color: var(--hi);
  }

  .group {
    padding: 10px 16px 6px;
  }

  /* Which kind the rows under it are; tighter under another track's name. */
  .kind {
    flex-shrink: 0;
    padding: 8px 16px 4px;
    font-family: var(--mono);
    font-size: 9.5px;
    letter-spacing: 0.12em;
    color: var(--lab);
  }

  .kind.sub {
    padding-top: 2px;
  }

  .who {
    color: var(--txt);
  }

  .state {
    border: 1px solid currentColor;
    padding: 0 4px;
    font-size: 9px;
    letter-spacing: 0.08em;
    color: var(--lab);
  }

  /* The app's "running" green, as on the track list's dots. */
  .state.running {
    color: var(--oktx);
  }

  .state.stalled,
  .state.died {
    color: var(--warn);
  }

  /* One line each: what it watches, its log's last line. */
  .one {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .note.full {
    white-space: pre-wrap;
  }

  /* A conductor's own job on this track leads nowhere: a row, not a control. */
  .main:disabled {
    cursor: default;
    opacity: 1;
  }

  .tname {
    padding: 6px 16px 4px;
    font-size: 11.5px;
    color: var(--txt);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row {
    display: flex;
    align-items: flex-start;
    border-top: 1px solid var(--lineq);
    cursor: default;
  }

  .row:hover {
    background: var(--sel);
  }

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 9px 4px 9px 16px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--dim);
  }

  .top {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 6px;
    font-size: 10.5px;
    letter-spacing: 0.04em;
    color: var(--lab);
  }

  .id {
    color: var(--lab);
  }

  .when {
    color: var(--hi);
  }

  .when.cond {
    color: var(--txt);
    overflow-wrap: anywhere;
  }

  .when.late,
  .tag {
    color: var(--warn);
  }

  .tag {
    border: 1px solid currentColor;
    padding: 0 4px;
    font-size: 9px;
    letter-spacing: 0.08em;
  }

  .msg {
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--txt);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    overflow-wrap: anywhere;
    white-space: pre-line;
  }

  .msg.full {
    display: block;
    -webkit-line-clamp: unset;
    line-clamp: unset;
  }

  .note {
    font-size: 10px;
    color: var(--lab);
    overflow-wrap: anywhere;
  }

  .note.warn {
    color: var(--warn);
  }

  .side {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
    padding: 8px 12px 8px 4px;
    flex-shrink: 0;
  }

  /* The conversation line's own cancel button. */
  .cancel {
    padding: 2px 8px;
    background: transparent;
    border: 1px solid var(--lineq);
    color: var(--lab);
    font-size: 10px;
  }

  .cancel:hover {
    color: var(--hi);
    border-color: var(--acc);
  }

  .more {
    width: 22px;
    height: 20px;
    background: transparent;
    border: 0;
    color: var(--lab);
    font-size: 11px;
    line-height: 1;
  }

  .more:hover {
    color: var(--hi);
  }
</style>
