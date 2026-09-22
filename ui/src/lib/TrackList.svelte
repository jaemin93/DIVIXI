<script lang="ts">
  import { store, agentLabel } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import SplitHandle from "./SplitHandle.svelte";

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
      name: "ACP 브리지",
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
  <SplitHandle edge="right" width={store.trackListWidth} min={200} max={480} reset={264} label="Tracks 열 너비" onchange={(px, persist) => store.setTrackListWidth(px, persist)} />
  <div class="head">
    <span class="mlab">Tracks</span>
    <span class="grow"></span>
    <button class="btn new" disabled title="곧: 낙서에서 승격하거나 직접 만들기">+ TRACK</button>
    <button class="x" onclick={() => store.setTrackList(false)} aria-label="목록 닫기" title="목록 닫기">
      <Icon name="collapse" />
    </button>
  </div>

  <input class="search" type="text" bind:value={query} placeholder="Track 검색" aria-label="Track 검색" />

  <div class="list">
    {#each shown as t (t.id)}
      <div class="track" class:on={store.view === "track"}>
        <button
          class="chev mono"
          onclick={() => (open = { ...open, [t.id]: !open[t.id] })}
          aria-label={open[t.id] ? "레인 접기" : "레인 펼치기"}
        >
          {open[t.id] ? "▾" : "▸"}
        </button>
        <button class="pick" onclick={() => (store.view = "track")}>
          <span class="dot" class:pulse={t.live} style="background: {t.live ? 'var(--ok)' : 'var(--idle)'}"></span>
          <span class="name">{t.name}</span>
          <span class="mono count">{t.runs}</span>
        </button>
      </div>

      {#if open[t.id]}
        {#each t.lanes as lane (lane.name)}
          <div class="lane">
            <span
              class="dot"
              class:pulse={lane.status === "running"}
              style="background: {lane.status === 'running' ? 'var(--ok)' : 'var(--idle)'}"
            ></span>
            <span class="mono name">{lane.name}</span>
            <span class="mono meta">{agentLabel(lane.agent)}</span>
            <span class="mono count">{lane.runs}</span>
          </div>
        {/each}
      {/if}
    {/each}
    {#if shown.length === 0}
      <div class="mono empty">없음</div>
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
    height: 30px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 16px 0 44px;
    font-size: 11px;
    color: var(--dim);
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
