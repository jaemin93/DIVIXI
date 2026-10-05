<script lang="ts">
  import { store } from "./lib/store.svelte";
  import Rail from "./lib/Rail.svelte";
  import TrackList from "./lib/TrackList.svelte";
  import Timeline from "./lib/Timeline.svelte";
  import Composer from "./lib/Composer.svelte";
  import Workspace from "./lib/Workspace.svelte";
  import Setup from "./lib/Setup.svelte";
  import PhoneDialog from "./lib/PhoneDialog.svelte";
  import TagDialog from "./lib/TagDialog.svelte";
  import Settings from "./lib/Settings.svelte";
  import WorkerView from "./lib/WorkerView.svelte";
  import TrackForm from "./lib/TrackForm.svelte";
  import TrackHeader from "./lib/TrackHeader.svelte";
  import DesignView from "./lib/DesignView.svelte";
  import KnowledgeView from "./lib/KnowledgeView.svelte";
  import GraphAgentPanel from "./lib/GraphAgentPanel.svelte";
  import RoutineList from "./lib/RoutineList.svelte";
  import RoutineView from "./lib/RoutineView.svelte";
  import WindowChrome from "./lib/WindowChrome.svelte";
  import Unreachable from "./lib/Unreachable.svelte";
  import NotPaired from "./lib/NotPaired.svelte";
  import ErrorToasts from "./lib/ErrorToasts.svelte";
  import CrashBanner from "./lib/CrashBanner.svelte";
  import UpdateBanner from "./lib/UpdateBanner.svelte";
  import Connecting from "./lib/Connecting.svelte";
  import { startup } from "./lib/startup.svelte";
  import { waiting } from "./lib/startup";
  import { inTauri, instance, overWeb, session } from "./lib/ipc.svelte";
  import { ZOOM_STEP } from "./lib/store.svelte";
  import { visibleFrame } from "./lib/viewport";
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";

  /** Side columns slide open and shut; not for those who asked for less motion. */
  const reduced = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const side = { axis: "x" as const, duration: reduced ? 0 : 200, easing: cubicOut };

  /** A phone-sized screen: no room for the track list beside a new track's form. */
  const narrowQuery = typeof matchMedia === "function" ? matchMedia("(max-width: 640px)") : null;
  let narrow = $state(narrowQuery?.matches ?? false);
  narrowQuery?.addEventListener("change", (e) => (narrow = e.matches));

  // In a phone's browser the soft keyboard covers the page instead of
  // shrinking it. The shell follows the part still visible, so the message
  // box and the newest message stay above the keyboard. The app's own
  // window has no soft keyboard and is left as it is.
  $effect(() => {
    const vv = overWeb ? window.visualViewport : null;
    if (!vv) return;
    const root = document.documentElement;
    const fit = () => {
      const f = visibleFrame(vv, window.innerHeight);
      root.style.setProperty("--vvh", `${f.height}px`);
      root.style.setProperty("--vvtop", `${f.top}px`);
    };
    fit();
    vv.addEventListener("resize", fit);
    vv.addEventListener("scroll", fit);
    return () => {
      vv.removeEventListener("resize", fit);
      vv.removeEventListener("scroll", fit);
    };
  });

  /** The track list sits beside every view but settings, once a track exists. */
  const listShown = $derived(
    store.view !== "settings" && store.view !== "design" && store.view !== "knowledge" && store.view !== "routines" && !(narrow && store.view === "new-track") && store.trackListOpen && store.tracks.length > 0,
  );

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

<!-- The webview's own right-click menu (reload, inspect, …) is not the app's:
     it never opens. The app's menus open where they belong. -->
<svelte:window onkeydown={onKey} oncontextmenu={(e) => e.preventDefault()} />

<!-- Errors from anywhere in the app, over whatever is on screen. Mounted
     once, outside every view, so none of them has to carry its own. -->
<ErrorToasts />

<!-- And, on the launch after a panic, what happened to the run before this
     one. It stays until it is closed: unlike an error, it is already over. -->
<CrashBanner />

<!-- And, once someone has asked, that a release is out or that its installer
     is downloaded and checked. Same place, same manners; nothing puts it there
     but a press (src-tauri/src/update.rs). The app's window only: a phone has
     no installer. -->
{#if inTauri}<UpdateBanner />{/if}

{#if session.arriving}
  <!-- A code was just scanned and is being redeemed. Blank rather than the
       not-paired page: this phone is about to be paired, and saying it is
       not would be wrong for as long as it took to read. -->
  <div class="waiting"></div>
{:else if !session.ready}
  <!-- A browser at this address with no pairing. Said plainly, instead of a
       shell whose every pane fails to load. -->
  <NotPaired />
{:else if instance.error}
  <div class="shell" class:web={overWeb}>
    <WindowChrome />
    <Unreachable />
  </div>
{:else if waiting(startup.phase)}
  <!-- Reaching the instance, or reading its tracks: not yet the app, and
       above all not its first-track screen, which says there are none. -->
  <div class="shell" class:web={overWeb}>
    {#if inTauri}<WindowChrome />{/if}
    <Connecting />
  </div>
{:else}
<div class="shell" class:web={overWeb}>
  {#if inTauri}<WindowChrome />{/if}
  {#if store.setupOpen}
    <Setup />
  {/if}
  {#if store.tagDialog}
    <TagDialog />
  {/if}
  {#if store.phoneDialog}
    <PhoneDialog onclose={() => (store.phoneDialog = false)} />
  {/if}
  <div class="body">
    <Rail />
    <div class="stage">
    <div class="row">
    <!-- One track list for every view, so switching views does not replay its slide. -->
    {#if listShown}
      <div class="sidebox" transition:slide={side}><TrackList /></div>
    {/if}
    {#if store.view === "settings"}
      <Settings />
    {:else if store.view === "design"}
      <DesignView />
    {:else if store.view === "knowledge"}
      <KnowledgeView />
      <!-- The graph's conversation: a column of the layout, beside the whole knowledge page. -->
      {#if store.graphChatOpen && store.graphChat}
        <GraphAgentPanel />
      {/if}
    {:else if store.view === "routines"}
      <RoutineList />
      <RoutineView />
    {:else if store.view === "worker"}
      <WorkerView />
    {:else if store.view === "new-track" || !store.currentTrack}
      <TrackForm />
    {:else if store.view === "edit-track"}
      {#key store.currentTrack.id}
        <TrackForm track={store.currentTrack} />
      {/key}
    {:else}
      <main>
        <TrackHeader track={store.currentTrack} />
        <Timeline />
        <Composer />
      </main>
      {#if store.panelOpen}
        <div class="sidebox" transition:slide={side}><Workspace /></div>
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
{/if}

<style>
  .waiting {
    min-height: 100dvh;
    background: var(--bg);
  }

  .shell {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }

  /* A browser: the strip above the soft keyboard (set from visualViewport
     above), or the dynamic viewport before that is known. */
  .shell.web {
    position: fixed;
    left: 0;
    right: 0;
    top: var(--vvtop, 0px);
    height: var(--vvh, 100dvh);
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

  /* Holds a side column while it slides; the column keeps its own width. */
  .sidebox {
    flex-shrink: 0;
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
</style>
