<script lang="ts">
  import { store, agentLabel, TRACK_COLORS, type TrackSort } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import ItemMenu from "./ItemMenu.svelte";
  import SplitHandle from "./SplitHandle.svelte";
  import { t } from "./i18n.svelte";
  import { whenLabel, whenFull, WINDOWS } from "./time";

  /**
   * Second column: every track, newest activity first, each unfolding into
   * its lanes (Workspace > Track > Lane). The open track's lanes are shown
   * unless folded by hand. A right click on a track opens its menu: rename,
   * tags, colour, settings, delete — after Kiro Crew's session menu.
   */
  let query = $state("");
  let folded = $state<Record<string, boolean>>({});

  const live = (r: { status: string }) => r.status === "running" || r.status === "connecting";

  // Lanes are whatever the conductor has opened in a track, from the runs.
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
          busy: runs.some((r) => r.lane === "conductor" && live(r)),
          lastAt: Math.max(tr.updated_at, ...runs.map((r) => r.startedAt)),
          lanes: store.laneNamesIn(tr.id).map((name) => {
            const laneRuns = runs.filter((r) => r.lane === name);
            return {
              name,
              agent: laneRuns.at(-1)?.agent ?? tr.agent,
              live: laneRuns.some(live),
              runs: laneRuns.length,
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
    if (store.openDecisions(tr.id) > 0) return "var(--warn)";
    return store.isActive(tr.id) ? "var(--ok)" : "var(--idle)";
  }
  function dotTitle(tr: Row): string {
    const waiting = store.openDecisions(tr.id);
    const session = store.isActive(tr.id) ? t("track.active") : t("track.inactive");
    return waiting > 0 ? `${t("tracks.decisionsOpen", { n: waiting })} · ${session}` : session;
  }

  function toggle(id: string) {
    folded = { ...folded, [id]: isOpen(id) };
  }

  // ----- context menu (shared with drafts: ItemMenu) -----
  type Menu = { id: string; x: number; y: number };
  let menu = $state<Menu | null>(null);

  const menuTrack = $derived.by(() => {
    const m = menu;
    return m ? tracks.find((x) => x.id === m.id) : undefined;
  });

  function openMenu(e: MouseEvent, id: string) {
    e.preventDefault();
    menu = { id, x: e.clientX, y: e.clientY };
  }

  function closeMenu() {
    menu = null;
  }

  /** Whether a click happened inside `el`, as it was when the click was dispatched. */
  function inside(e: MouseEvent, el: HTMLElement | undefined): boolean {
    return !!el && e.composedPath().includes(el);
  }

  function onDocClick(e: MouseEvent) {
    if (filterOpen && !inside(e, filterEl)) filterOpen = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") filterOpen = false;
  }

  async function openSettings() {
    if (!menu) return;
    const id = menu.id;
    closeMenu();
    await store.selectTrack(id);
    store.view = "edit-track";
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

<!-- One track and, when unfolded, its lanes. -->
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
          <button class="chev" onclick={() => toggle(tr.id)} aria-label={isOpen(tr.id) ? t("tracks.foldLanes") : t("tracks.unfoldLanes")}>
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
            <span class="mono when" title={whenFull(tr.lastAt, store.lang)}>{whenLabel(tr.lastAt, store.now, store.lang)}</span>
          </span>
          <span class="main">
            <span class="name">{tr.name}</span>
            <span class="mono count">{tr.runs}</span>
          </span>
        </button>
      </div>

      {#if isOpen(tr.id)}
        {#each tr.lanes as lane (lane.name)}
          <button
            class="lane"
            class:on={store.view === "lane" && store.track === tr.id && store.openLane === lane.name}
            onclick={async () => {
              if (store.track !== tr.id) await store.selectTrack(tr.id);
              store.openLaneView(lane.name);
            }}
          >
            <span class="dot" class:pulse={lane.live} style="background: {lane.live ? 'var(--ok)' : 'var(--idle)'}"></span>
            <span class="mono name">{lane.name}</span>
            <span class="mono meta">{agentLabel(lane.agent)}</span>
            <span class="mono count">{lane.runs}</span>
          </button>
        {/each}
      {/if}
{/snippet}

{#if menu && menuTrack}
  {@const tr = menuTrack}
  <ItemMenu
    x={menu.x}
    y={menu.y}
    name={tr.name}
    color={tr.color}
    busy={tr.busy}
    busyNote={t("track.busyNote")}
    deleteNote={t("track.deleteNote")}
    onclose={closeMenu}
    onrename={(name) => store.updateTrack(tr.id, { name }).then(() => {})}
    ontags={() => (store.tagDialog = tr.id)}
    oncolor={(color) => store.updateTrack(tr.id, { color }).then(() => {})}
    ondelete={() => store.deleteTrack(tr.id)}
  >
    {#snippet top()}
      {#if store.isActive(tr.id)}
        <button class="item" role="menuitem" disabled={tr.busy} onclick={() => { closeMenu(); store.closeConductor(tr.id); }}>{t("track.deactivate")}</button>
      {:else}
        <button class="item" role="menuitem" disabled={store.conductorOpening === tr.id} onclick={() => { closeMenu(); store.openConductor(tr.id); }}>{t("track.activate")}</button>
      {/if}
    {/snippet}
    {#snippet middle()}
      <button class="item" role="menuitem" onclick={openSettings}>{t("track.settings")}</button>
    {/snippet}
  </ItemMenu>
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
    outline: none;
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

  .lane {
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

  .lane:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .lane.on {
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

  .lane .name {
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

  .meta {
    font-size: 10px;
    color: var(--lab);
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

</style>
