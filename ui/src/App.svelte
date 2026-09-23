<script lang="ts">
  import { store } from "./lib/store.svelte";
  import Rail from "./lib/Rail.svelte";
  import TrackList from "./lib/TrackList.svelte";
  import Timeline from "./lib/Timeline.svelte";
  import Composer from "./lib/Composer.svelte";
  import Workspace from "./lib/Workspace.svelte";
  import Setup from "./lib/Setup.svelte";
  import TagDialog from "./lib/TagDialog.svelte";
  import Settings from "./lib/Settings.svelte";
  import LaneView from "./lib/LaneView.svelte";
  import TrackForm from "./lib/TrackForm.svelte";
  import TrackHeader from "./lib/TrackHeader.svelte";
  import WindowChrome from "./lib/WindowChrome.svelte";
  import { ZOOM_STEP } from "./lib/store.svelte";

  // Browser-style zoom keys, app-wide.
  function onKey(e: KeyboardEvent) {
    if (!(e.ctrlKey || e.metaKey)) return;
    if (e.key === "`") {
      // Ctrl+`: the terminal panel, as in VS Code and Kiro.
      e.preventDefault();
      store.setTerminal(!store.termOpen);
      return;
    }
    // Zoom keys stay with the shell while it has focus.
    if ((e.target as HTMLElement | null)?.closest?.(".xterm")) return;
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
  {#if store.tagDialog}
    <TagDialog />
  {/if}
  <div class="body">
    <Rail />
    <div class="stage">
    <div class="row">
    {#if store.view === "settings"}
      <Settings />
    {:else if store.view === "lane"}
      {#if store.trackListOpen}
        <TrackList />
      {/if}
      <LaneView />
    {:else if store.view === "new-track" || !store.currentTrack}
      {#if store.trackListOpen && store.tracks.length > 0}
        <TrackList />
      {/if}
      <TrackForm />
    {:else if store.view === "edit-track"}
      {#if store.trackListOpen}
        <TrackList />
      {/if}
      {#key store.currentTrack.id}
        <TrackForm track={store.currentTrack} />
      {/key}
    {:else}
      {#if store.trackListOpen}
        <TrackList />
      {/if}
      <main>
        <TrackHeader track={store.currentTrack} />
        <Timeline />
        <Composer />
      </main>
      {#if store.panelOpen}
        <Workspace />
      {/if}
    {/if}
    </div>
    {#if store.termMounted}
      <!-- Loaded on first use: xterm.js is as big as the rest of the app. -->
      {#await import("./lib/TerminalPanel.svelte") then panel}
        <panel.default />
      {/await}
    {/if}
    </div>
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

  /* Right of the rail: the views on top, the terminal panel under them. */
  .stage {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }

  .row {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background-image: radial-gradient(var(--dot) 1px, transparent 1px);
    background-size: 22px 22px;
  }
</style>
