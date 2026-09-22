<script lang="ts">
  import { store } from "./lib/store.svelte";
  import Rail from "./lib/Rail.svelte";
  import TrackList from "./lib/TrackList.svelte";
  import Timeline from "./lib/Timeline.svelte";
  import Composer from "./lib/Composer.svelte";
  import Inspector from "./lib/Inspector.svelte";
  import Setup from "./lib/Setup.svelte";
  import Settings from "./lib/Settings.svelte";
  import LaneView from "./lib/LaneView.svelte";
  import WindowChrome from "./lib/WindowChrome.svelte";
  import { ZOOM_STEP } from "./lib/store.svelte";

  // Browser-style zoom keys, app-wide.
  function onKey(e: KeyboardEvent) {
    if (!(e.ctrlKey || e.metaKey)) return;
    if (e.key === "=" || e.key === "+") {
      e.preventDefault();
      store.setZoom(store.zoom + ZOOM_STEP);
    } else if (e.key === "-") {
      e.preventDefault();
      store.setZoom(store.zoom - ZOOM_STEP);
    } else if (e.key === "0") {
      e.preventDefault();
      store.setZoom(100);
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="shell">
  <WindowChrome />
  {#if store.setupOpen}
    <Setup />
  {/if}
  <div class="body">
    <Rail />
    {#if store.view === "settings"}
      <Settings />
    {:else if store.view === "lane"}
      {#if store.trackListOpen}
        <TrackList />
      {/if}
      <LaneView />
    {:else}
      {#if store.trackListOpen}
        <TrackList />
      {/if}
      <main>
        <header>
          <div class="mlab">#01 / Track</div>
          <h1 class="serif">ACP 브리지</h1>
          <p>devterm 오케스트라를 ACP 위로 올린다. 레인은 워크트리로 격리하고, 보고는 스키마로 강제한다.</p>
        </header>
        <Timeline />
        <Composer />
      </main>
      {#if store.openRun}
        <Inspector run={store.openRun} />
      {/if}
    {/if}
  </div>
</div>

<style>
  .shell {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }

  .body {
    flex: 1;
    display: flex;
    min-height: 0;
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

  h1 {
    margin: 8px 0 0;
    font-weight: 400;
    font-size: 36px;
    line-height: 1.1;
    color: var(--hi);
  }

  p {
    margin: 9px 0 0;
    font-size: 13px;
    line-height: 1.6;
    color: var(--dim);
    max-width: 620px;
  }
</style>
