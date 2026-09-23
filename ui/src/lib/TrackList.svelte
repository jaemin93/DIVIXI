<script lang="ts">
  import { store, agentLabel } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import SplitHandle from "./SplitHandle.svelte";
  import { t } from "./i18n.svelte";

  /**
   * Second column: every track, newest activity first, each unfolding into
   * its lanes (Workspace > Track > Lane). The open track's lanes are shown
   * unless folded by hand.
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
          runs: runs.length,
          live: runs.some(live),
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
      return !q || tr.name.toLowerCase().includes(q) || tr.intent.toLowerCase().includes(q);
    }),
  );

  function isOpen(id: string): boolean {
    return id in folded ? !folded[id] : id === store.track;
  }

  function toggle(id: string) {
    folded = { ...folded, [id]: isOpen(id) };
  }
</script>

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
      <div class="track" class:on={store.view === "track" && store.track === tr.id}>
        <button class="chev mono" onclick={() => toggle(tr.id)} aria-label={isOpen(tr.id) ? t("tracks.foldLanes") : t("tracks.unfoldLanes")}>
          {isOpen(tr.id) ? "▾" : "▸"}
        </button>
        <button class="pick" onclick={() => store.selectTrack(tr.id)} title={tr.intent}>
          <span class="dot" class:pulse={tr.live} style="background: {tr.live ? 'var(--ok)' : 'var(--idle)'}"></span>
          <span class="name">{tr.name}</span>
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
    height: 40px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 0 0 8px;
    border-left: 2px solid transparent;
    color: var(--dim);
  }

  .track:hover {
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
    border-left-color: var(--acc);
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
    border-left-color: var(--acc);
    background: var(--sel);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
