<script lang="ts">
  import { store, agentLabel, TRACK_COLORS } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import SplitHandle from "./SplitHandle.svelte";
  import { t } from "./i18n.svelte";

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
          color: tr.color,
          tags: tr.tags,
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

  const shown = $derived(
    tracks.filter((tr) => {
      const q = query.trim().toLowerCase();
      return !q || tr.name.toLowerCase().includes(q) || tr.intent.toLowerCase().includes(q) || tr.tags.some((g) => g.toLowerCase().includes(q));
    }),
  );

  function isOpen(id: string): boolean {
    return id in folded ? !folded[id] : id === store.track;
  }

  function toggle(id: string) {
    folded = { ...folded, [id]: isOpen(id) };
  }

  // ----- context menu -----
  type Menu = { id: string; x: number; y: number };
  let menu = $state<Menu | null>(null);
  let mode = $state<"" | "rename" | "confirm">("");
  let draft = $state("");
  let menuEl = $state<HTMLDivElement>();
  let draftInput = $state<HTMLInputElement>();

  const menuTrack = $derived.by(() => {
    const m = menu;
    return m ? tracks.find((x) => x.id === m.id) : undefined;
  });

  function openMenu(e: MouseEvent, id: string) {
    e.preventDefault();
    mode = "";
    draft = "";
    menu = { id, x: e.clientX, y: e.clientY };
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

  function onDocClick(e: MouseEvent) {
    if (menu && menuEl && !menuEl.contains(e.target as Node)) closeMenu();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && menu) closeMenu();
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
    closeMenu();
    await store.deleteTrack(id);
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

  <input class="search" type="text" bind:value={query} placeholder={t("tracks.search")} aria-label={t("tracks.search")} />

  <div class="list">
    {#each shown as tr (tr.id)}
      <div
        class="track"
        class:on={store.view === "track" && store.track === tr.id}
        class:menued={menu?.id === tr.id}
        style="border-left-color: {tr.color || 'transparent'}"
        oncontextmenu={(e) => openMenu(e, tr.id)}
        role="presentation"
      >
        <button class="chev mono" onclick={() => toggle(tr.id)} aria-label={isOpen(tr.id) ? t("tracks.foldLanes") : t("tracks.unfoldLanes")}>
          {isOpen(tr.id) ? "▾" : "▸"}
        </button>
        <button class="pick" onclick={() => store.selectTrack(tr.id)} title={tr.intent}>
          <span class="dot" class:pulse={tr.live} style="background: {tr.live ? 'var(--ok)' : 'var(--idle)'}"></span>
          <span class="lines">
            {#if tr.tags.length}
              <span class="tags">
                {#each tr.tags as tag (tag)}
                  <span class="mono chip" style="color: {store.tagColor(tag)}"><span class="tdot" style="background: {store.tagColor(tag)}"></span>{tag}</span>
                {/each}
              </span>
            {/if}
            <span class="name">{tr.name}</span>
          </span>
          <span class="mono count">{tr.runs}</span>
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
    {/each}
    {#if shown.length === 0}
      <div class="mono empty">{t("tracks.none")}</div>
    {/if}
  </div>
</aside>

{#if menu && menuTrack}
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

  .search {
    margin: 10px 12px 6px;
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

  .track {
    width: 100%;
    min-height: 40px;
    display: flex;
    align-items: center;
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
    height: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
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

  .chev {
    width: 18px;
    height: 18px;
    background: transparent;
    border: 0;
    color: var(--lab);
    font-size: 10px;
    padding: 0;
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

  .lines {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
  }

  /* Tags sit in a small line above the name, each in its own colour. */
  .tags {
    display: flex;
    gap: 8px;
    overflow: hidden;
    white-space: nowrap;
    line-height: 1;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 9px;
    letter-spacing: 0.08em;
    flex-shrink: 0;
  }

  .tdot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
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
    outline: none;
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
