<script lang="ts">
  import { store, agentLabel, TRACK_COLORS, type TrackSort } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import SplitHandle from "./SplitHandle.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel, whenFull, WINDOWS } from "./time";

  /**
   * Second column: every track, newest activity first, each unfolding into
   * its workers (Workspace > Track > Worker). The open track's workers are shown
   * unless folded by hand. A right click on a track opens its menu: rename,
   * tags, colour, settings, delete — after Kiro Crew's session menu.
   */
  let query = $state("");
  let folded = $state<Record<string, boolean>>({});
  /** Whether a track's finished workers are shown; see `doneOpen`. */
  let doneFolded = $state<Record<string, boolean>>({});
  /** What the last tidy-up did, under the track it was done on. */
  let tidied = $state<Record<string, string>>({});

  const live = (r: { status: string }) => r.status === "running" || r.status === "connecting";

  // Workers are whatever the conductor has opened in a track, from the runs.
  const tracks = $derived(
    store.tracks
      .map((tr) => {
        const runs = store.runs.filter((r) => r.track === tr.id);
        return {
          id: tr.id,
          name: tr.name,
          intent: tr.intent,
          agent: tr.agent,
          color: tr.color,
          tags: tr.tags,
          createdAt: tr.created_at,
          runs: runs.length,
          live: runs.some(live),
          busy: runs.some((r) => r.session === "conductor" && live(r)),
          lastAt: Math.max(tr.updated_at, ...runs.map((r) => r.startedAt)),
          parked: store.parked[tr.id] ?? 0,
          workers: store.workerNamesIn(tr.id).map((name) => {
            const workerRuns = runs.filter((r) => r.session === name);
            const session = store.workerSession(tr.id, name);
            return {
              name,
              track: tr.id,
              agent: session?.agent ?? workerRuns.at(-1)?.agent ?? tr.agent,
              model: session?.model ?? "",
              live: workerRuns.some(live) || !!session?.running,
              // Its agent process is alive, whether or not it has work.
              open: !!session?.open,
              runs: workerRuns.length,
            };
          }),
        };
      })
      .sort((a, b) => b.lastAt - a.lastAt),
  );

  // ----- filter, sort, fold -----
  const filter = $derived(store.trackFilter);
  const filterActive = $derived(filter.running || filter.active || filter.recent !== "" || filter.tags.length > 0);
  let filterOpen = $state(false);
  let filterEl = $state<HTMLDivElement>();
  let showDormant = $state(false);

  const sorters: Record<TrackSort, (a: (typeof tracks)[number], b: (typeof tracks)[number]) => number> = {
    recent: (a, b) => b.lastAt - a.lastAt,
    oldest: (a, b) => a.lastAt - b.lastAt,
    "created-desc": (a, b) => b.createdAt - a.createdAt,
    "created-asc": (a, b) => a.createdAt - b.createdAt,
    az: (a, b) => a.name.localeCompare(b.name),
    za: (a, b) => b.name.localeCompare(a.name),
  };

  const matched = $derived(
    tracks
      .filter((tr) => {
        const q = query.trim().toLowerCase();
        if (q && !(tr.name.toLowerCase().includes(q) || tr.intent.toLowerCase().includes(q) || tr.tags.some((g) => g.toLowerCase().includes(q)))) return false;
        if (filter.running && !tr.live) return false;
        if (filter.active && !store.isActive(tr.id)) return false;
        if (filter.recent && store.now - tr.lastAt > WINDOWS[filter.recent]) return false;
        if (filter.tags.length && !filter.tags.every((g) => tr.tags.includes(g))) return false;
        return true;
      })
      .sort(sorters[filter.sort]),
  );

  /** Tracks idle past the fold threshold sit under a header at the bottom; the open one never folds. */
  const foldMs = $derived(filter.fold * 24 * 60 * 60 * 1000);
  const shown = $derived(foldMs ? matched.filter((tr) => tr.id === store.track || store.now - tr.lastAt <= foldMs) : matched);
  const dormant = $derived(foldMs ? matched.filter((tr) => tr.id !== store.track && store.now - tr.lastAt > foldMs) : []);

  /** How many tracks carry a tag, for the filter's tag rows. */
  function tagCount(tag: string): number {
    return store.tracks.filter((x) => x.tags.includes(tag)).length;
  }

  function toggleTagFilter(tag: string) {
    const tags = filter.tags.includes(tag) ? filter.tags.filter((x) => x !== tag) : [...filter.tags, tag];
    void store.setTrackFilter({ tags });
  }

  function isOpen(id: string): boolean {
    return id in folded ? !folded[id] : id === store.track;
  }

  type Row = (typeof tracks)[number];
  function dotColor(tr: Row): string {
    if (tr.live || store.conductorOpening === tr.id) return "var(--ok)";
    if (store.openDecisions(tr.id) > 0 || tr.parked > 0) return "var(--warn)";
    return store.isActive(tr.id) ? "var(--ok)" : "var(--idle)";
  }
  function dotTitle(tr: Row): string {
    const waiting = store.openDecisions(tr.id);
    const session = store.isActive(tr.id) ? t("track.active") : t("track.inactive");
    const lines = [waiting > 0 ? t("tracks.decisionsOpen", { n: waiting }) : "", tr.parked > 0 ? t("tracks.parked", { n: tr.parked }) : "", session];
    return lines.filter(Boolean).join(" · ");
  }

  function toggle(id: string) {
    folded = { ...folded, [id]: isOpen(id) };
  }

  type Worker = Row["workers"][number];

  /** Workers in a turn right now; these are never folded away. */
  const working = (tr: Row): Worker[] => tr.workers.filter((w) => w.live);
  /** Workers that have stopped. They are what piles up. */
  const done = (tr: Row): Worker[] => tr.workers.filter((w) => !w.live);

  /** A short list of finished workers stays open; a long one folds itself. */
  function doneOpen(tr: Row): boolean {
    return tr.id in doneFolded ? !doneFolded[tr.id] : done(tr).length <= 3;
  }

  function toggleDone(tr: Row) {
    doneFolded = { ...doneFolded, [tr.id]: doneOpen(tr) };
  }

  /** Finished workers whose agent session is still open: what tidying closes. */
  const tidyable = (tr: Row): number => done(tr).filter((w) => w.open).length;

  async function tidy(tr: Row) {
    const { closed, error } = await store.tidyWorkers(tr.id);
    tidied = { ...tidied, [tr.id]: error || (closed.length ? t("tracks.tidied", { n: closed.length }) : t("tracks.tidiedNone")) };
    setTimeout(() => {
      const { [tr.id]: _gone, ...rest } = tidied;
      tidied = rest;
    }, 4000);
  }

  /**
   * What a worker is running on, for the tooltip and the menu head.
   *
   * This stays out of the list line on purpose. The line was carrying the
   * agent name once and it was removed as noise, and mixing agents does not
   * change that: a name repeated down ten rows says nothing, and showing it
   * only on the odd one out would make labels appear and disappear as
   * workers open, which reads worse than either. What does change is that
   * the answer now has two parts, so both are here — where it is asked for
   * rather than always on screen.
   */
  function workerAgent(w: Worker): string {
    const model = w.model ? store.choiceName(store.optionsOf(w.agent).find((o) => o.category === "model"), w.model) : "";
    return model ? `${agentLabel(w.agent)} · ${model}` : agentLabel(w.agent);
  }

  /** The whole of a worker row, for the tooltip: name, agent, how it is doing. */
  function workerTitle(w: Worker): string {
    const state = w.live ? t("worker.working") : w.open ? t("worker.sessionOpen") : t("worker.sessionClosed");
    return `${w.name} · ${workerAgent(w)} · ${state}`;
  }

  // ----- context menu -----
  /** A right click on a track, or on one of its workers. */
  type Menu = { id: string; worker?: string; x: number; y: number };
  let menu = $state<Menu | null>(null);
  let mode = $state<"" | "rename" | "confirm">("");
  let draft = $state("");
  /** Why the core refused a delete, shown in the menu. */
  let deleteError = $state("");
  let menuEl = $state<HTMLDivElement>();
  let draftInput = $state<HTMLInputElement>();

  const menuTrack = $derived.by(() => {
    const m = menu;
    return m ? tracks.find((x) => x.id === m.id) : undefined;
  });

  function openMenu(e: MouseEvent, id: string, worker?: string) {
    e.preventDefault();
    e.stopPropagation();
    mode = "";
    deleteError = "";
    draft = "";
    menu = { id, worker, x: e.clientX, y: e.clientY };
  }

  const menuWorker = $derived.by(() => {
    const m = menu;
    return m?.worker ? menuTrack?.workers.find((w) => w.name === m.worker) : undefined;
  });

  async function stopWorker() {
    const w = menuWorker;
    if (!w) return;
    deleteError = "";
    const refused = await store.cancelWorker(w.track, w.name);
    if (refused) deleteError = refused;
    else closeMenu();
  }

  async function closeWorkerSession() {
    const w = menuWorker;
    if (!w) return;
    deleteError = "";
    const refused = await store.closeWorker(w.track, w.name);
    if (refused) deleteError = refused;
    else closeMenu();
  }

  async function removeWorker() {
    const w = menuWorker;
    if (!w) return;
    deleteError = "";
    const refused = await store.deleteWorker(w.track, w.name);
    if (refused) deleteError = refused;
    else closeMenu();
  }

  function closeMenu() {
    menu = null;
    mode = "";
  }

  /** Keep the menu inside the window once it has a size. */
  $effect(() => {
    if (!menu || !menuEl) return;
    const r = menuEl.getBoundingClientRect();
    const x = Math.min(menu.x, window.innerWidth - r.width - 8);
    const y = Math.min(menu.y, window.innerHeight - r.height - 8);
    if (x !== menu.x || y !== menu.y) menu = { ...menu, x: Math.max(8, x), y: Math.max(8, y) };
  });

  $effect(() => {
    if (mode === "rename") draftInput?.focus();
  });

  /** Whether a click happened inside `el`. The path is taken when the click
   *  is dispatched, so a button the click itself replaced (Delete turning
   *  into the confirm step) still counts as inside. */
  function inside(e: MouseEvent, el: HTMLElement | undefined): boolean {
    return !!el && e.composedPath().includes(el);
  }

  function onDocClick(e: MouseEvent) {
    if (menu && !inside(e, menuEl)) closeMenu();
    if (filterOpen && !inside(e, filterEl)) filterOpen = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (menu) closeMenu();
      filterOpen = false;
    }
  }

  function startRename() {
    draft = menuTrack?.name ?? "";
    mode = "rename";
  }

  function openTags() {
    if (!menu) return;
    store.tagDialog = menu.id;
    closeMenu();
  }

  async function commit(e: Event) {
    e.preventDefault();
    if (!menu) return;
    if (mode === "rename" && draft.trim()) await store.updateTrack(menu.id, { name: draft.trim() });
    closeMenu();
  }

  async function pickColor(color: string) {
    if (!menu) return;
    await store.updateTrack(menu.id, { color });
    closeMenu();
  }

  async function openSettings() {
    if (!menu) return;
    await store.selectTrack(menu.id);
    store.view = "edit-track";
    closeMenu();
  }

  async function remove() {
    if (!menu) return;
    const id = menu.id;
    deleteError = "";
    const refused = await store.deleteTrack(id);
    if (refused) deleteError = refused;
    else closeMenu();
  }
</script>

<svelte:document onclick={onDocClick} onkeydown={onKey} />

<aside style="width: {store.trackListWidth}px">
  <SplitHandle edge="right" width={store.trackListWidth} min={200} max={480} reset={264} label={t("tracks.width")} onchange={(px, persist) => store.setTrackListWidth(px, persist)} />
  <div class="head">
    <span class="mlab">{t("tracks.title")}</span>
    <span class="grow"></span>
    <button class="btn new" class:on={store.view === "new-track"} title={t("tracks.newTitle")} onclick={() => (store.view = "new-track")}>{t("tracks.new")}</button>
    <button class="x" onclick={() => store.setTrackList(false)} aria-label={t("tracks.close")} title={t("tracks.close")}>
      <Icon name="collapse" />
    </button>
  </div>

  <div class="searchrow" bind:this={filterEl}>
    <input class="search" type="text" bind:value={query} placeholder={t("tracks.search")} aria-label={t("tracks.search")} />
    <button class="fbtn" class:on={filterActive || filterOpen} onclick={() => (filterOpen = !filterOpen)} title={t("tracks.filter")} aria-haspopup="menu" aria-expanded={filterOpen}>
      <Icon name="filter" size={14} />
    </button>

    {#if filterOpen}
      <!-- The filter menu, after Kiro Crew's: what to show, in what order, what to fold, which tags. -->
      <div class="fmenu" role="menu" aria-label={t("tracks.filter")}>
        <div class="mlab-sm fhead">{t("tracks.filterTitle")}</div>
        <button class="fitem" class:on={filter.running} role="menuitemcheckbox" aria-checked={filter.running} onclick={() => store.setTrackFilter({ running: !filter.running })}>
          <span class="dot" style="background: var(--ok)"></span>{t("tracks.running")}
        </button>
        <button class="fitem" class:on={filter.active} role="menuitemcheckbox" aria-checked={filter.active} onclick={() => store.setTrackFilter({ active: !filter.active })}>
          <span class="dot" style="background: var(--idle)"></span>{t("tracks.active")}
        </button>
        <div class="fitem sub" class:on={filter.recent !== ""} role="menuitem" aria-haspopup="menu">
          <span>{t("tracks.recent")}{filter.recent ? ` · ${filter.recent}` : ""}</span>
          <span class="grow"></span>
          <span class="mono arrow">›</span>
          <div class="submenu" role="menu">
            <div class="subhead">{t("tracks.recentHead")}</div>
            {#each ["", "1h", "24h", "7d"] as w (w)}
              <button class="fitem" class:on={filter.recent === w} role="menuitemradio" aria-checked={filter.recent === w} onclick={() => store.setTrackFilter({ recent: w as typeof filter.recent })}>
                {w || t("tracks.recentAll")}<span class="grow"></span>{#if filter.recent === w}<span class="mono check">✓</span>{/if}
              </button>
            {/each}
          </div>
        </div>

        <div class="rule"></div>
        <div class="mlab-sm fhead">{t("tracks.sortTitle")}</div>
        {#each [["recent", t("tracks.sort.recent")], ["oldest", t("tracks.sort.oldest")], ["created-desc", t("tracks.sort.createdDesc")], ["created-asc", t("tracks.sort.createdAsc")], ["az", "A → Z"], ["za", "Z → A"]] as [id, label] (id)}
          <button class="fitem" class:on={filter.sort === id} role="menuitemradio" aria-checked={filter.sort === id} onclick={() => store.setTrackFilter({ sort: id as TrackSort })}>
            {label}<span class="grow"></span>{#if filter.sort === id}<span class="mono check">✓</span>{/if}
          </button>
        {/each}

        <div class="rule"></div>
        <div class="fitem sub" class:on={filter.fold > 0} role="menuitem" aria-haspopup="menu">
          <span>{t("tracks.fold")}{filter.fold > 0 ? ` · ${filter.fold}d` : ""}</span>
          <span class="grow"></span>
          <span class="mono arrow">›</span>
          <div class="submenu" role="menu">
            <div class="subhead">{t("tracks.foldHead")}</div>
            {#each [0, 1, 2, 7, 14] as d (d)}
              <button class="fitem" class:on={filter.fold === d} role="menuitemradio" aria-checked={filter.fold === d} onclick={() => store.setTrackFilter({ fold: d })}>
                {d === 0 ? t("tracks.foldOff") : `${d}d`}<span class="grow"></span>{#if filter.fold === d}<span class="mono check">✓</span>{/if}
              </button>
            {/each}
          </div>
        </div>

        <div class="rule"></div>
        <div class="mlab-sm fhead">{t("tracks.tagsTitle")}</div>
        <div class="taglist">
        {#each store.tagPool as tag (tag.name)}
          <button class="fitem" class:on={filter.tags.includes(tag.name)} role="menuitemcheckbox" aria-checked={filter.tags.includes(tag.name)} onclick={() => toggleTagFilter(tag.name)}>
            <span class="box" class:on={filter.tags.includes(tag.name)} style="border-color: {tag.color}; background: {filter.tags.includes(tag.name) ? tag.color : 'transparent'}"></span>
            <span class="mono">{tag.name}</span>
            <span class="grow"></span>
            <span class="mono count">{tagCount(tag.name)}</span>
          </button>
        {/each}
        {#if store.tagPool.length === 0}
          <div class="fitem static dim">{t("tracks.noTagsYet")}</div>
        {/if}
        </div>
        {#if filterActive}
          <div class="rule"></div>
          <button class="fitem" onclick={() => store.setTrackFilter({ running: false, active: false, recent: "", tags: [] })}>{t("tracks.clear")}</button>
        {/if}
      </div>
    {/if}
  </div>

  <div class="list">
    {#each shown as tr (tr.id)}
      {@render row(tr)}
    {/each}
    {#if dormant.length}
      <button class="dormant" onclick={() => (showDormant = !showDormant)}>
        <span class="mono chev">{showDormant ? "▾" : "▸"}</span>
        <span>{t("tracks.dormant")}</span>
        <span class="mono count">{dormant.length}</span>
      </button>
      {#if showDormant}
        {#each dormant as tr (tr.id)}
          {@render row(tr)}
        {/each}
      {/if}
    {/if}
    {#if shown.length === 0 && dormant.length === 0}
      <div class="mono empty">{t("tracks.none")}</div>
    {/if}
  </div>
</aside>

<!-- One track and, when unfolded, its workers. -->
{#snippet row(tr: (typeof tracks)[number])}
      <div
        class="track"
        class:on={store.view === "track" && store.track === tr.id}
        class:menued={menu?.id === tr.id}
        style="border-left-color: {tr.color || 'transparent'}"
        oncontextmenu={(e) => openMenu(e, tr.id)}
        role="presentation"
      >
        <!-- The session state over the fold toggle: grey closed, green active,
             pulsing while it works, orange while a decision waits on the human. -->
        <div class="side">
          <span
            class="dot"
            class:pulse={tr.live || store.conductorOpening === tr.id}
            style="background: {dotColor(tr)}"
            title={dotTitle(tr)}
          ></span>
          <button class="chev" onclick={() => toggle(tr.id)} aria-label={isOpen(tr.id) ? t("tracks.foldWorkers") : t("tracks.unfoldWorkers")}>
            {isOpen(tr.id) ? "▾" : "▸"}
          </button>
        </div>
        <button class="pick" onclick={() => store.selectTrack(tr.id)} title={tr.intent}>
          <!-- The small line: tags on the left, last activity on the right. -->
          <span class="top">
            <!-- Tags give way first: the run of chips is cut with an ellipsis; the time always shows. -->
            <span class="taglist" title={tr.tags.join(", ")}>
              {#each tr.tags as tag (tag)}
                <span class="mono chip" style="color: {store.tagColor(tag)}">{tag}</span>
              {/each}
            </span>
            <!-- Reports the conductor could not take. They are kept and go in
                 as soon as it can; until then this is what says they exist. -->
            {#if tr.parked > 0}<span class="mono parked" title={t("tracks.parked", { n: tr.parked })}>⏳{tr.parked}</span>{/if}
            <span class="mono when" title={whenFull(tr.lastAt, store.lang)}>{whenLabel(tr.lastAt, store.now, store.lang)}</span>
          </span>
          <span class="main">
            <span class="name">{tr.name}</span>
          </span>
        </button>
      </div>

      {#if isOpen(tr.id)}
        <!-- Workers at work stay in sight; the ones that finished fold away
             under a header, so a track that has run twenty of them still reads. -->
        {#each working(tr) as worker (worker.name)}
          {@render workerRow(tr, worker)}
        {/each}
        {#if done(tr).length}
          <div class="wgroup">
            <button class="ghead" onclick={() => toggleDone(tr)} aria-expanded={doneOpen(tr)} aria-label={doneOpen(tr) ? t("tracks.foldDone") : t("tracks.unfoldDone")}>
              <span class="mono chev">{doneOpen(tr) ? "▾" : "▸"}</span>
              <span>{t("tracks.workersDone")}</span>
              <span class="mono count">{done(tr).length}</span>
            </button>
            {#if tidyable(tr) > 0}
              <button class="tidy" onclick={() => tidy(tr)} title={t("tracks.tidyTitle")}>{t("tracks.tidy")}<span class="mono count">{tidyable(tr)}</span></button>
            {/if}
          </div>
          {#if tidied[tr.id]}
            <div class="mono tidied">{tidied[tr.id]}</div>
          {/if}
          {#if doneOpen(tr)}
            {#each done(tr) as worker (worker.name)}
              {@render workerRow(tr, worker)}
            {/each}
          {/if}
        {/if}
      {/if}
{/snippet}

<!-- One worker: a dot for its session, its name, its agent. Right click opens its menu. -->
{#snippet workerRow(tr: (typeof tracks)[number], worker: Worker)}
  <button
    class="worker"
    class:on={store.view === "worker" && store.track === tr.id && store.openWorker === worker.name}
    class:menued={menu?.id === tr.id && menu?.worker === worker.name}
    oncontextmenu={(e) => openMenu(e, tr.id, worker.name)}
    title={workerTitle(worker)}
    onclick={async () => {
      if (store.track !== tr.id) await store.selectTrack(tr.id);
      store.openWorkerView(worker.name);
    }}
  >
    <span class="dot" class:pulse={worker.live} class:hollow={!worker.open && !worker.live} style="background: {worker.live || worker.open ? 'var(--ok)' : 'var(--idle)'}"></span>
    <span class="mono name">{worker.name}</span>
  </button>
{/snippet}

{#if menu && menuWorker}
  <!-- The worker menu: stop it, close its session, or delete its record.
       Deleting is the only one that cannot be undone, so it sits under a
       rule, in the danger colour, behind a confirm step that says what goes
       and what stays. -->
  <div class="menu" bind:this={menuEl} style="left: {menu.x}px; top: {menu.y}px" role="menu" aria-label={t("worker.menu")}>
    <div class="mono whead">{menuWorker.name}<span class="wagent">{workerAgent(menuWorker)}</span></div>
    <button class="item" role="menuitem" disabled={!menuWorker.live} title={t("worker.stopNote")} onclick={stopWorker}>{t("worker.stop")}</button>
    <button class="item" role="menuitem" disabled={!menuWorker.open || menuWorker.live} title={t("worker.closeNote")} onclick={closeWorkerSession}>{t("worker.closeSession")}</button>
    <div class="rule"></div>
    {#if mode === "confirm"}
      <div class="mono note">{menuWorker.live ? t("worker.busyNote") : t("worker.deleteNote")}</div>
      {#if !menuWorker.live}<div class="mono note keeps">{t("worker.deleteKeeps")}</div>{/if}
      {#if deleteError}<div class="mono note err">{deleteError}</div>{/if}
      <div class="acts pad">
        <button class="btn sm" type="button" onclick={() => (mode = "")}>{t("track.cancel")}</button>
        <span class="grow"></span>
        <button class="btn sm danger" type="button" disabled={menuWorker.live} onclick={removeWorker}>{t("worker.confirmDelete")}</button>
      </div>
    {:else}
      {#if deleteError}<div class="mono note err">{deleteError}</div>{/if}
      <button class="item danger" role="menuitem" onclick={() => (mode = "confirm")}>{t("worker.delete")}</button>
    {/if}
  </div>
{:else if menu && menuTrack}
  <div class="menu" bind:this={menuEl} style="left: {menu.x}px; top: {menu.y}px" role="menu" aria-label={t("track.menu")}>
    {#if mode === "rename"}
      <form class="inline" onsubmit={commit}>
        <span class="mlab-sm">{t("track.rename")}</span>
        <input type="text" bind:this={draftInput} bind:value={draft} maxlength="80" />
        <div class="acts">
          <span class="grow"></span>
          <button class="btn sm" type="button" onclick={() => (mode = "")}>{t("track.cancel")}</button>
          <button class="btn sm btn-acc" type="submit" disabled={!draft.trim()}>{t("newtrack.save")}</button>
        </div>
      </form>
    {:else}
      {#if store.isActive(menuTrack.id)}
        <button class="item" role="menuitem" disabled={menuTrack.busy} onclick={() => { const id = menuTrack!.id; closeMenu(); store.closeConductor(id); }}>{t("track.deactivate")}</button>
      {:else}
        <button class="item" role="menuitem" disabled={store.conductorOpening === menuTrack.id} onclick={() => { const id = menuTrack!.id; closeMenu(); store.openConductor(id); }}>{t("track.activate")}</button>
      {/if}
      <div class="rule"></div>
      <button class="item" role="menuitem" onclick={startRename}>{t("track.rename")}</button>
      <button class="item" role="menuitem" onclick={openTags}>{t("track.tags")}</button>
      <button class="item" role="menuitem" onclick={openSettings}>{t("track.settings")}</button>
      <div class="rule"></div>
      <div class="colors" role="group" aria-label={t("track.color")}>
        <button class="sw none" class:on={!menuTrack.color} title={t("track.noColor")} onclick={() => pickColor("")}></button>
        {#each TRACK_COLORS as c (c)}
          <button class="sw" class:on={menuTrack.color === c} style="background: {c}" title={c} onclick={() => pickColor(c)}></button>
        {/each}
      </div>
      <div class="rule"></div>
      {#if mode === "confirm"}
        <div class="mono note">{menuTrack.busy ? t("track.busyNote") : t("track.deleteNote")}</div>
        {#if deleteError}<div class="mono note err">{deleteError}</div>{/if}
        <div class="acts pad">
          <button class="btn sm" type="button" onclick={() => (mode = "")}>{t("track.cancel")}</button>
          <span class="grow"></span>
          <button class="btn sm danger" type="button" disabled={menuTrack.busy} onclick={remove}>{t("track.confirmDelete")}</button>
        </div>
      {:else}
        <button class="item danger" role="menuitem" onclick={() => (mode = "confirm")}>{t("track.delete")}</button>
      {/if}
    {/if}
  </div>
{/if}

<style>
  aside {
    position: relative;
    flex-shrink: 0;
    border-right: 1px solid var(--line);
    background: var(--rail);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .head {
    height: 44px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px 0 16px;
    border-bottom: 1px solid var(--line);
  }

  .grow {
    flex: 1;
  }

  .new {
    height: 26px;
    padding: 0 9px;
  }

  .new.on {
    color: var(--hi);
    border-color: var(--acc);
  }

  .x {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
  }

  .x:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .searchrow {
    position: relative;
    display: flex;
    gap: 6px;
    margin: 10px 12px 6px;
  }

  .fbtn {
    width: 32px;
    height: 32px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--lab);
  }

  .fbtn:hover,
  .fbtn.on {
    color: var(--hi);
    border-color: var(--acc);
  }

  /* The filter menu hangs under the search row. */
  .fmenu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    right: 0;
    z-index: 30;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 4px 0 6px;
  }

  .taglist {
    max-height: 30vh;
    overflow-y: auto;
  }

  /* A row with a submenu: hovering opens it to the right, as Kiro does. */
  .fitem.sub {
    position: relative;
  }

  .arrow {
    color: var(--lab);
    font-size: 12px;
  }

  .submenu {
    display: none;
    position: absolute;
    top: -5px;
    left: calc(100% - 6px);
    min-width: 200px;
    z-index: 31;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 4px 0 6px;
    color: var(--dim);
  }

  .fitem.sub:hover,
  .fitem.sub:focus-within {
    color: var(--hi);
    background: var(--sel);
  }

  .fitem.sub:hover > .submenu,
  .fitem.sub:focus-within > .submenu {
    display: block;
  }

  .subhead {
    padding: 8px 14px 6px;
    font-size: 11px;
    color: var(--lab);
    white-space: nowrap;
  }

  .fhead {
    padding: 10px 14px 4px;
  }

  .fitem {
    width: 100%;
    min-height: 30px;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 4px 14px;
    background: transparent;
    border: 0;
    text-align: left;
    font-size: 12px;
    color: var(--dim);
  }

  .fitem:not(.static):hover {
    color: var(--hi);
    background: var(--sel);
  }

  .fitem.on {
    color: var(--hi);
  }

  .fitem.dim {
    color: var(--lab);
    font-size: 11px;
  }

  .fitem .check {
    color: var(--acct);
    font-size: 11px;
  }

  .fitem .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .fitem .box {
    width: 10px;
    height: 10px;
    border: 1px solid;
    flex-shrink: 0;
  }

  .dormant {
    width: 100%;
    height: 34px;
    margin-top: 6px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 16px 0 10px;
    background: transparent;
    border: 0;
    border-top: 1px solid var(--lineq);
    text-align: left;
    font-size: 12px;
    color: var(--lab);
  }

  .dormant:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .search {
    flex: 1;
    min-width: 0;
    height: 32px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 12px;
    padding: 0 10px;
  }

  .search:focus {
    border-color: var(--acc);
  }

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 0 12px;
  }

  /* Two lines per track: agent, tags and time above; dot, name and runs
     below. The chevron sits with the name line. */
  .track {
    width: 100%;
    min-height: 52px;
    display: flex;
    align-items: flex-end;
    gap: 4px;
    padding: 0 0 0 8px;
    border-left: 2px solid transparent;
    color: var(--dim);
  }

  .track:hover,
  .track.menued {
    color: var(--hi);
    background: var(--sel);
  }

  .pick {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    padding: 0 16px 0 4px;
    background: transparent;
    border: 0;
    text-align: left;
    font-size: 13px;
    color: inherit;
  }

  .track.on {
    color: var(--hi);
    background: var(--sel);
  }

  /* Left of the name: the session dot on the small line, the fold toggle on the name line. */
  .side {
    align-self: stretch;
    width: 22px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
  }

  .side .dot {
    margin-top: 11px;
  }

  .chev {
    margin-top: auto;
    width: 22px;
    height: 36px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--lab);
    font-size: 14px;
    line-height: 1;
    padding: 0;
  }

  .chev:hover {
    color: var(--hi);
  }

  .worker {
    width: 100%;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 16px 0 44px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    text-align: left;
    font-size: 11px;
    color: var(--dim);
  }

  .worker:hover,
  .worker.menued {
    color: var(--hi);
    background: var(--sel);
  }

  /* Finished workers, under a header that folds them away and offers to
     close their sessions in one go. */
  .wgroup {
    display: flex;
    align-items: center;
    padding-right: 10px;
  }

  .ghead {
    flex: 1;
    min-width: 0;
    height: 26px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 30px;
    background: transparent;
    border: 0;
    text-align: left;
    font-size: 10px;
    color: var(--lab);
  }

  .ghead:hover,
  .tidy:hover {
    color: var(--hi);
  }

  .ghead .chev {
    width: auto;
    height: auto;
    margin: 0;
    font-size: 10px;
  }

  .tidy {
    height: 20px;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 0 6px;
    background: transparent;
    border: 1px solid var(--lineq);
    color: var(--lab);
    font-size: 10px;
  }

  .tidied {
    padding: 2px 16px 4px 44px;
    font-size: 10px;
    color: var(--lab);
  }

  /* A worker whose session has been closed: the record is still there, the
     agent process is not. */
  .dot.hollow {
    background: transparent !important;
    border: 1px solid var(--idle);
  }

  .worker.on {
    color: var(--hi);
    background: var(--sel);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .worker .name {
    flex: 1;
  }

  .main {
    height: 36px;
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .main .name {
    flex: 1;
  }

  /* The small line above the name: agent, tags, and when the track last moved. */
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 0 0 0;
    margin-bottom: -4px;
    overflow: hidden;
    white-space: nowrap;
    line-height: 1.3;
  }

  .parked {
    font-size: 9px;
    letter-spacing: 0.04em;
    color: var(--warn);
    flex-shrink: 0;
  }

  .when {
    font-size: 9px;
    letter-spacing: 0.04em;
    color: var(--lab);
    flex-shrink: 0;
  }

  /* The chips are 9px but the list would inherit the row's 13px line box and
     sit on its baseline, a pixel or two under the time and the dot. */
  .taglist {
    font-size: 9px;
    line-height: 1.3;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chip {
    font-size: 9px;
    letter-spacing: 0.08em;
  }

  .chip + .chip {
    margin-left: 8px;
  }

  .count {
    font-size: 10px;
    color: var(--lab);
  }

  .empty {
    padding: 18px 16px;
    font-size: 11px;
    color: var(--lab);
  }

  /* The track menu: a fixed sheet at the pointer, hairline and square. */
  .menu {
    position: fixed;
    z-index: 40;
    min-width: 210px;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 4px 0;
  }

  .item {
    width: 100%;
    height: 30px;
    display: flex;
    align-items: center;
    padding: 0 14px;
    background: transparent;
    border: 0;
    text-align: left;
    font-size: 12px;
    color: var(--dim);
  }

  .item:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .item.danger {
    color: var(--acct);
  }

  /* The worker the menu is about, so the pointer's target stays named once
     the menu covers the row. */
  .whead {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 6px 14px 4px;
    font-size: 10px;
    color: var(--lab);
  }

  /* Which agent this worker runs on: not in the list line, but here. */
  .wagent {
    font-size: 9px;
    letter-spacing: 0.06em;
    color: var(--lab);
    opacity: 0.8;
  }

  .item:disabled {
    color: var(--lab);
    cursor: default;
  }

  .rule {
    height: 1px;
    background: var(--line);
    margin: 4px 0;
  }

  .colors {
    display: flex;
    gap: 7px;
    padding: 6px 14px;
  }

  .sw {
    width: 16px;
    height: 16px;
    border: 1px solid transparent;
    padding: 0;
  }

  .sw.none {
    background: transparent;
    border-color: var(--lines);
    background-image: linear-gradient(135deg, transparent 46%, var(--acct) 46%, var(--acct) 54%, transparent 54%);
  }

  .sw.on {
    outline: 1px solid var(--hi);
    outline-offset: 2px;
  }

  .note {
    padding: 6px 14px 2px;
    font-size: 10px;
    color: var(--dim);
    max-width: 260px;
  }

  .note.err {
    color: var(--acct);
  }

  /* What a delete leaves alone, said as plainly as what it takes. */
  .note.keeps {
    color: var(--lab);
  }

  .inline {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 12px 10px;
    width: 260px;
  }

  .inline input {
    height: 30px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 12px;
    padding: 0 9px;
  }

  .inline input:focus {
    border-color: var(--acc);
  }

  .acts {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .acts.pad {
    padding: 4px 12px 8px;
  }

  .btn.sm {
    height: 24px;
    padding: 0 8px;
  }

  .btn.danger:not(:disabled) {
    color: var(--acct);
    border-color: var(--acct);
  }
</style>
