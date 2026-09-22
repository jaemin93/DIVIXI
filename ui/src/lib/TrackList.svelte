<script lang="ts">
  import { store, agentLabel } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import SplitHandle from "./SplitHandle.svelte";
  import { t } from "./i18n.svelte";

  /**
   * Second column: every track, each unfolding into its lanes.
   *
   * Phase 0 has one track and one lane, so the list is short; the shape is
   * the domain's (Workspace > Track > Lane), not a placeholder for it.
   */
  let query = $state("");
  let open = $state<Record<string, boolean>>({ acp: true });

  // Lanes are whatever the conductor has opened in this track, from the runs.
  const tracks = $derived([
    {
      id: "acp",
      name: t("track.placeholderName"),
      runs: store.runs.length,
      live: !!store.activeRun,
      lanes: store.laneNames.map((name) => {
        const runs = store.runs.filter((r) => r.lane === name);
        const live = runs.some((r) => r.status === "running" || r.status === "connecting");
        return {
          name,
          agent: runs.at(-1)?.agent ?? store.agent,
          status: live ? "running" : "idle",
          runs: runs.length,
        };
      }),
    },
  ]);

  const shown = $derived(
    tracks.filter((t) => !query.trim() || t.name.toLowerCase().includes(query.trim().toLowerCase())),
  );
</script>

<aside style="width: {store.trackListWidth}px">
  <SplitHandle edge="right" width={store.trackListWidth} min={200} max={480} reset={264} label={t("tracks.width")} onchange={(px, persist) => store.setTrackListWidth(px, persist)} />
  <div class="head">
    <span class="mlab">{t("tracks.title")}</span>
    <span class="grow"></span>
    <button class="btn new" disabled title={t("tracks.newSoon")}>{t("tracks.new")}</button>
    <button class="x" onclick={() => store.setTrackList(false)} aria-label={t("tracks.close")} title={t("tracks.close")}>
      <Icon name="collapse" />
    </button>
  </div>

  <input class="search" type="text" bind:value={query} placeholder={t("tracks.search")} aria-label={t("tracks.search")} />

  <div class="list">
    {#each shown as tr (tr.id)}
      <div class="track" class:on={store.view === "track"}>
        <button
          class="chev mono"
          onclick={() => (open = { ...open, [tr.id]: !open[tr.id] })}
          aria-label={open[tr.id] ? t("tracks.foldLanes") : t("tracks.unfoldLanes")}
        >
          {open[tr.id] ? "▾" : "▸"}
        </button>
        <button class="pick" onclick={() => (store.view = "track")}>
          <span class="dot" class:pulse={tr.live} style="background: {tr.live ? 'var(--ok)' : 'var(--idle)'}"></span>
          <span class="name">{tr.name}</span>
          <span class="mono count">{tr.runs}</span>
        </button>
      </div>

      {#if open[tr.id]}
        {#each tr.lanes as lane (lane.name)}
          <button class="lane" class:on={store.view === "lane" && store.openLane === lane.name} onclick={() => store.openLaneView(lane.name)}>
            <span
              class="dot"
              class:pulse={lane.status === "running"}
              style="background: {lane.status === 'running' ? 'var(--ok)' : 'var(--idle)'}"
            ></span>
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
