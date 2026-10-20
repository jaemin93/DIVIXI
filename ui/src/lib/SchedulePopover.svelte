<script lang="ts">
  import { store, type Watched } from "./store.svelte";
  import Popover from "./Popover.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import { t } from "./i18n.svelte";
  import { whenFull } from "./time";
  import { isCondition, lateness, message, rule } from "./scheduleList";
  import { isOpen, jobState, lastMoved, watches, when, type JobState } from "./jobList";
  import { clock, isFiltered, visible, type WatchKind, type WatchOrder } from "./watchFilter";

  /**
   * The track's schedules and background jobs, one row each, beside the
   * panel toggle at the top right: what will wake the conductor and when,
   * then what the app is watching for it. Shown only while the track has
   * one or the other. Other tracks' follow under their own names, read when
   * the popover opens; the `schedules` and `jobs` events keep every list
   * live after that.
   *
   * A row's time and state sit in columns of their own, its kind in an
   * icon (a clock, a loop for a repetition, an eye for a condition, a
   * terminal for a job). The filter at the top right keeps one kind or both
   * and orders them (`watchFilter.ts`); it is remembered like the track
   * list's.
   *
   * The conversation shows none of this: it is all here. A schedule's row
   * opens and closes on a click; a job's opens the worker that registered
   * it (the track first, for another track's), and its chevron opens the
   * row. A conductor's own job on this track has no worker to open, so its
   * row opens instead. Cancelling a schedule and stopping a job's watch are
   * here.
   */
  let open = $state(false);
  let expanded = $state<number[]>([]);
  /** Expanded job rows: their own ids, which may be a schedule's too. */
  let jobsExpanded = $state<number[]>([]);
  let menuOpen = $state(false);
  let menuBox = $state<HTMLDivElement>();

  const narrowQuery = typeof matchMedia === "function" ? matchMedia("(max-width: 640px)") : null;
  let narrow = $state(narrowQuery?.matches ?? false);
  $effect(() => {
    if (!narrowQuery) return;
    const on = () => (narrow = narrowQuery.matches);
    narrowQuery.addEventListener("change", on);
    return () => narrowQuery.removeEventListener("change", on);
  });

  $effect(() => {
    if (!open) menuOpen = false;
  });

  const filter = $derived(store.watchFilter);
  const mine = (list: Watched[], track: string) => list.filter((x) => x.track === track);
  /** Everything this track has, unfiltered: whether the button shows, and its count. */
  const all = $derived(visible(mine(store.schedules, store.track), mine(store.jobs, store.track), store.now, { kind: "all", order: "newest" }));
  const live = $derived(all.schedules.length + all.jobs.filter(isOpen).length);
  const here = $derived(visible(mine(store.schedules, store.track), mine(store.jobs, store.track), store.now, filter));
  const others = $derived.by(() => {
    const out: { track: string; name: string; schedules: Watched[]; jobs: Watched[] }[] = [];
    for (const x of store.tracks) {
      if (x.id === store.track) continue;
      const v = visible(mine(store.schedules, x.id), mine(store.jobs, x.id), store.now, filter);
      if (v.schedules.length || v.jobs.length) out.push({ track: x.id, name: x.name, ...v });
    }
    return out;
  });
  const nothing = $derived(!here.schedules.length && !here.jobs.length && !others.length);
  /** Kind headings only when both kinds are on: with one, the filter already says which. */
  const headings = $derived(filter.kind === "all");
  const locale = $derived(store.lang === "ko" ? "ko-KR" : "en-US");
  const tr = (key: string, vars?: Record<string, string | number>) => t(key as Parameters<typeof t>[0], vars);
  const at = (ms: number) => clock(ms, store.now, tr);

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

  /** Whether a job's row leads anywhere: to the worker that registered it, or to the track of a conductor's own on another track. */
  function jobGoes(j: Watched): boolean {
    return !!j.worker || j.track !== store.track;
  }

  async function openJob(j: Watched) {
    if (!jobGoes(j)) {
      expandJob(j.id);
      return;
    }
    open = false;
    if (j.track !== store.track) await store.selectTrack(j.track);
    if (j.worker) store.openWorkerView(j.worker);
  }

  function setKind(kind: WatchKind) {
    store.setWatchFilter({ kind });
  }

  function setOrder(order: WatchOrder) {
    store.setWatchFilter({ order });
  }

  /** A click in the panel outside the filter menu closes the menu, not the panel. */
  function onPanelClick(e: MouseEvent) {
    if (menuOpen && menuBox && !e.composedPath().includes(menuBox)) menuOpen = false;
  }

  /** A schedule's kind icon: a loop for a repetition, an eye for a condition, else a clock. */
  function scheduleIcon(s: Watched): IconName {
    if (isCondition(s)) return "eye";
    return s.kind === "at" || !s.kind ? "clock" : "routines";
  }

  /** A job's state, as its dot and word show it. */
  function tone(st: JobState): string {
    return st === "running" ? "ok" : st === "stalled" ? "warn" : st === "died" ? "bad" : "quiet";
  }

  const kinds: [WatchKind, string][] = $derived([
    ["all", t("schedules.kindAll")],
    ["schedules", t("schedules.title")],
    ["jobs", t("jobs.title")],
  ]);
  const orders: [WatchOrder, string][] = $derived([
    ["newest", t("schedules.newest")],
    ["oldest", t("schedules.oldest")],
  ]);
  const filterLabel = $derived(isFiltered(filter) ? `${t("schedules.filter")} · ${t("tracks.filterOn")}` : t("schedules.filter"));
</script>

{#if all.schedules.length || all.jobs.length}
  <Popover bind:open align="right" width={narrow ? 10_000 : 400} label={t("schedules.titleBoth")}>
    {#snippet trigger()}
      <button class="tog" class:on={open} type="button" title={t("schedules.titleBoth")} aria-label={t("schedules.titleBoth")} aria-haspopup="dialog" aria-expanded={open} onclick={toggle}>
        <Icon name="clock" size={14} />
        {#if live}<span class="n mono" aria-hidden="true">{live}</span>{/if}
      </button>
    {/snippet}

    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="wp" class:menu={menuOpen} onclick={onPanelClick}>
      <div class="hd">
        <span class="title">{t("schedules.titleBoth")}</span>
        <span class="grow"></span>
        <div class="fwrap" bind:this={menuBox}>
          <button class="fbtn" class:on={menuOpen} class:set={isFiltered(filter)} type="button" onclick={() => (menuOpen = !menuOpen)} title={filterLabel} aria-label={filterLabel} aria-haspopup="menu" aria-expanded={menuOpen}>
            <Icon name="filter" size={13} />
          </button>
          {#if menuOpen}
            <!-- The track list's filter menu (ListSearch): a choice that is on has the accent bar, bold text and a ✓. -->
            <div class="fmenu" role="menu" aria-label={t("schedules.filter")}>
              <div class="mlab-sm fhead">{t("schedules.kindTitle")}</div>
              {#each kinds as [id, label] (id)}
                <button class="fitem" class:on={filter.kind === id} role="menuitemradio" aria-checked={filter.kind === id} onclick={() => setKind(id)}>
                  {#if id === "schedules"}<Icon name="clock" size={12} />{:else if id === "jobs"}<Icon name="terminal" size={12} />{:else}<span class="iconpad"></span>{/if}
                  <span>{label}</span>
                  <span class="grow"></span>{#if filter.kind === id}<span class="mono check" aria-hidden="true">✓</span>{/if}
                </button>
              {/each}
              <div class="rule"></div>
              <div class="mlab-sm fhead">{t("schedules.orderTitle")}</div>
              {#each orders as [id, label] (id)}
                <button class="fitem" class:on={filter.order === id} role="menuitemradio" aria-checked={filter.order === id} onclick={() => setOrder(id)}>
                  <span class="iconpad"></span>
                  <span>{label}</span>
                  <span class="grow"></span>{#if filter.order === id}<span class="mono check" aria-hidden="true">✓</span>{/if}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      </div>

      {@render block(here.schedules, here.jobs, headings)}
      {#if others.length}
        <div class="others mlab-sm">{t("schedules.others")}</div>
        {#each others as g (g.track)}
          <div class="tname" title={g.name}>{g.name}</div>
          {@render block(g.schedules, g.jobs, false)}
        {/each}
      {/if}
      {#if nothing}<div class="empty">{t("schedules.none")}</div>{/if}
    </div>
  </Popover>
{/if}

<!-- One track's schedules, then its jobs; each kind named only when both are on. -->
{#snippet block(items: Watched[], jobs: Watched[], named: boolean)}
  {#if items.length}
    {#if named}<div class="kind"><Icon name="clock" size={12} /><span>{t("schedules.title")}</span><span class="mono cnt">{items.length}</span></div>{/if}
    {#each items as s (s.id)}
      {@render item(s)}
    {/each}
  {/if}
  {#if jobs.length}
    {#if named}<div class="kind"><Icon name="terminal" size={12} /><span>{t("jobs.title")}</span><span class="mono cnt">{jobs.length}</span></div>{/if}
    {#each jobs as j (j.id)}
      {@render jobItem(j)}
    {/each}
  {/if}
{/snippet}

{#snippet item(s: Watched)}
  {@const late = lateness(s, store.now)}
  {@const r = rule(s, tr, locale)}
  {@const text = message(s)}
  {@const full = expanded.includes(s.id)}
  {@const c = s.next_at != null ? at(s.next_at) : null}
  <div class="row" class:full class:nostate={!late} id="schedule-row-{s.id}">
    <button class="main" type="button" aria-expanded={full} onclick={() => expand(s.id)} title={full ? t("schedules.less") : t("schedules.more")}>
      <span class="when mono" title={s.next_at != null ? whenFull(s.next_at, store.lang) : undefined}>
        {#if c}
          <span class="t" class:late={late === "overdue"}>{c.time}</span>{#if c.day}<span class="d">{c.day}</span>{/if}
        {:else}
          <span class="d">{t("schedules.condition")}</span>
        {/if}
      </span>
      <span class="body">
        <span class="ttl" class:clamp={!full}>{text}</span>
        <span class="meta mono">
          <Icon name={scheduleIcon(s)} size={11} />
          <span class="mtext">{r ?? t("schedules.once")}</span>
          <span class="id">#{s.id}</span>
        </span>
        {#if s.note}<span class="note mono" class:warn={late === "fired-late"}>{s.note}</span>{/if}
      </span>
    </button>
    {#if late}<div class="info"><span class="st warn"><i class="dot"></i>{t("schedules.late")}</span></div>{/if}
    {@render acts(t("schedules.cancel"), "close", () => store.cancelSchedule(s.id), full, () => expand(s.id))}
  </div>
{/snippet}

{#snippet jobItem(j: Watched)}
  {@const st = jobState(j)}
  {@const since = when(j)}
  {@const moved = lastMoved(j)}
  {@const full = jobsExpanded.includes(j.id)}
  {@const goes = jobGoes(j)}
  {@const c = since != null ? at(since) : null}
  {@const details = [j.what, j.pid ? `pid ${j.pid}` : "", j.log ? tr("jobs.log", { path: j.log }) : "", j.done_pattern ? tr("jobs.donePattern", { p: j.done_pattern }) : ""].filter(Boolean).join("\n")}
  <div class="row" class:full class:over={!isOpen(j)} id="job-row-{j.id}">
    <button class="main" type="button" aria-expanded={goes ? undefined : full} onclick={() => openJob(j)} title={!goes ? (full ? t("schedules.less") : t("schedules.more")) : j.worker ? t("jobs.openWorker") : t("schedules.openTrack")}>
      <span class="when mono" title={since != null ? whenFull(since, store.lang) : undefined}>
        {#if c}<span class="t">{c.time}</span><span class="d">{c.day || (isOpen(j) ? t("jobs.startedShort") : t("jobs.endedShort"))}</span>{/if}
      </span>
      <span class="body">
        <span class="ttl" class:clamp={!full}>{full ? details : j.what || watches(j)}</span>
        <span class="meta mono">
          <Icon name="terminal" size={11} />
          <span class="mtext">{[j.worker || t("jobs.conductor"), !full && j.what && watches(j) !== j.what ? watches(j).replace(`${j.what} · `, "") : ""].filter(Boolean).join(" · ")}</span>
          <span class="id">#{j.id}</span>
        </span>
        {#if j.last_line && (full || !j.note?.includes(j.last_line))}<span class="note mono" class:one={!full}>{full ? tr("jobs.lastLine", { line: j.last_line }) : j.last_line}</span>{/if}
        <!-- The core keeps a stall's note when the log moves again: a running job's would be stale. -->
        {#if j.note && st !== "running"}<span class="note mono" class:warn={st === "died" || st === "stalled"}>{j.note}</span>{/if}
      </span>
    </button>
    <div class="info">
      <span class="st {tone(st)}"><i class="dot" class:pulse={st === "running"}></i>{tr(`jobs.${st}`)}</span>
      {#if moved != null}<span class="moved mono" title={t("jobs.movedTitle")}>{tr("jobs.moved", { time: at(moved).time })}</span>{/if}
    </div>
    {@render acts(isOpen(j) ? t("jobs.unwatch") : null, "eyeoff", () => store.unwatchJob(j.id), full, () => expandJob(j.id))}
  </div>
{/snippet}

<!--
  A row's actions, the same two slots at the same place on every row: the
  row's own action (cancel a schedule, stop watching a job), then its
  chevron. A row with no action (a job that is over) keeps the slot empty,
  so every chevron lines up. Icons, each with its words as tooltip and
  label; always shown, so no row looks different from its neighbour.
-->
{#snippet acts(label: string | null, icon: IconName, act: () => void, full: boolean, toggle: () => void)}
  <div class="acts">
    <span class="slot">
      {#if label}
        <button class="act" type="button" onclick={act} title={label} aria-label={label}><Icon name={icon} size={12} /></button>
      {/if}
    </span>
    <span class="slot">
      <button class="act chev" type="button" aria-expanded={full} onclick={toggle} title={full ? t("schedules.less") : t("schedules.more")} aria-label={full ? t("schedules.less") : t("schedules.more")}>
        <Icon name={full ? "up" : "down"} size={12} />
      </button>
    </span>
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

  /* Its full height, not the panel's: the panel scrolls, and the sticky head
     is held inside this box, so a box cut to the panel would let it go. */
  .wp {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    padding-bottom: 6px;
  }

  /* Room for the filter menu when the list is short. */
  .wp.menu {
    min-height: 270px;
  }

  .wp > * {
    flex-shrink: 0;
  }

  /* The head stays at the top while the list scrolls under it. */
  .hd {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 44px;
    padding: 0 10px 0 16px;
    background: var(--card);
    border-bottom: 1px solid var(--line);
  }

  .title {
    font-size: 13px;
    font-weight: 500;
    color: var(--hi);
  }

  .grow {
    flex: 1;
  }

  .fwrap {
    position: relative;
  }

  /* The track list's filter button (ListSearch), a size down. */
  .fbtn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--lab);
  }

  .fbtn:hover,
  .fbtn.on {
    color: var(--hi);
    border-color: var(--acc);
  }

  .fbtn.set {
    color: var(--hi);
    background: var(--accbg);
    border-color: var(--accln);
  }

  .fmenu {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 5;
    width: 190px;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 4px 0 6px;
    box-shadow: 0 8px 24px color-mix(in srgb, var(--bg) 60%, transparent);
  }

  .fhead {
    padding: 8px 14px 4px;
  }

  .rule {
    height: 1px;
    background: var(--line);
    margin: 4px 0;
  }

  .fitem {
    width: 100%;
    min-height: 30px;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 4px 14px 4px 12px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    text-align: left;
    font-size: 12px;
    color: var(--dim);
  }

  .fitem:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .fitem.on {
    color: var(--hi);
    border-left-color: var(--acc);
    background: var(--accbg);
    font-weight: 600;
  }

  .fitem.on:hover {
    background: var(--sel);
  }

  .check {
    color: var(--acct);
    font-size: 11px;
  }

  .iconpad {
    width: 12px;
  }

  /* A kind's heading: its icon, its name, how many. */
  .kind {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 12px 16px 4px;
    font-family: var(--mono);
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--lab);
  }

  .cnt {
    margin-left: auto;
    letter-spacing: 0;
  }

  .others {
    margin-top: 10px;
    padding: 10px 16px 2px;
    border-top: 1px solid var(--line);
  }

  .tname {
    padding: 6px 16px 2px;
    font-size: 12px;
    color: var(--txt);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty {
    padding: 22px 16px;
    font-size: 12px;
    color: var(--lab);
    text-align: center;
  }

  /* ----- a row: time | what | state and actions ----- */

  /* What it is | its state | its actions: the last two fixed widths, so
     every row's state and buttons sit at the same place. */
  .row {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 66px 64px;
    align-items: start;
  }

  .main {
    min-width: 0;
    display: grid;
    grid-template-columns: 50px 1fr;
    column-gap: 12px;
    align-items: start;
    padding: 9px 8px 9px 16px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--dim);
  }

  .main:focus-visible,
  .act:focus-visible,
  .fbtn:focus-visible,
  .fitem:focus-visible {
    outline: 1px solid var(--focus);
    outline-offset: -1px;
  }

  .when {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-top: 1px;
  }

  .t {
    font-size: 12px;
    color: var(--hi);
  }

  .t.late {
    color: var(--warn);
  }

  .d {
    font-size: 9.5px;
    color: var(--lab);
    letter-spacing: 0.04em;
  }

  .body {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .ttl {
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--txt);
    overflow-wrap: anywhere;
    white-space: pre-line;
  }

  .ttl.clamp {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: 10px;
    color: var(--lab);
  }

  .mtext {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .id {
    margin-left: auto;
    flex-shrink: 0;
    color: var(--lab);
    opacity: 0.8;
  }

  .note {
    font-size: 10px;
    color: var(--lab);
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }

  .note.one {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .note.warn {
    color: var(--warn);
  }


  /* The state and, for a running job, when its log last moved: right-aligned in a column of their own. */
  .info {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 5px;
    padding: 10px 0 9px;
    min-width: 0;
  }

  /* A row with no state lends its state column to its text; the actions keep theirs. */
  .row.nostate .main {
    grid-column: 1 / span 2;
  }

  .info {
    grid-column: 2;
  }

  /* Two square slots, at the top right of every row. */
  .acts {
    grid-column: 3;
    display: flex;
    justify-content: flex-end;
    gap: 4px;
    padding: 6px 10px 0 6px;
  }

  .slot {
    width: 24px;
    height: 24px;
    flex-shrink: 0;
  }

  /* Both buttons, the action and the chevron, alike: a square icon button,
     quiet until pointed at. */
  .act {
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    background: transparent;
    border: 1px solid transparent;
    color: var(--lab);
  }

  .act:hover {
    color: var(--hi);
    border-color: var(--lines);
    background: var(--card);
  }

  /* Cancelling and stopping a watch take something away: their hover says so. */
  .act:not(.chev):hover {
    color: var(--deltx);
    border-color: var(--deltx);
  }

  /* A state: a dot and a word, in the app's own colours. */
  .st {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-family: var(--mono);
    font-size: 10px;
    white-space: nowrap;
    color: var(--lab);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
  }

  .st.ok {
    color: var(--oktx);
  }

  .st.ok .dot {
    background: var(--ok);
  }

  .st.warn {
    color: var(--warn);
  }

  .st.bad {
    color: var(--deltx);
  }

  .st.quiet .dot {
    background: var(--idle);
  }

  .dot.pulse {
    animation: pulse 1.6s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }

  .moved {
    font-size: 9.5px;
    color: var(--lab);
  }






  /* A row is a list item, as the track list's: no box, a tint under the
     pointer, a quieter tint and a hairline at the left while it is open. */
  .row:hover,
  .row:focus-within {
    background: var(--sel);
  }

  .row.full {
    background: var(--cardq);
    box-shadow: inset 2px 0 0 var(--lines);
  }

  .row.over .ttl {
    color: var(--dim);
  }

</style>
