<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { store } from "./store.svelte";
  import { i18n } from "./i18n.svelte";
  import { loadExcalidraw, meaningCounts, type BoardApi, type DesignDelta, type DesignScene, type ExcalidrawModule } from "./excalidraw";

  /**
   * A design's board: Excalidraw itself, mounted as a React root. What the
   * human draws is saved a moment after they stop (the core merges it
   * element by element with anything the agent drew meanwhile); what the
   * agent draws arrives as a `design` event and is reconciled in as an
   * undoable step, so Ctrl+Z takes it back.
   */
  let { id }: { id: string } = $props();

  let host = $state<HTMLDivElement>();
  let failed = $state("");

  /** Wait this long after the last change before saving. */
  const SAVE_AFTER = 600;

  /** Draw Excalidraw with the current theme and language; set once it is loaded. */
  let render: (() => void) | null = null;

  // The app's theme and language follow into the board.
  $effect(() => {
    void store.theme;
    void i18n.lang;
    render?.();
  });

  onMount(() => {
    let disposed = false;
    let ex: ExcalidrawModule | null = null;
    let api: BoardApi | null = null;
    let root: import("react-dom/client").Root | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    /** The scene version and file count last saved (or loaded). */
    let savedVersion = -1;
    let savedFiles = 0;

    const save = async () => {
      clearTimeout(timer);
      timer = undefined;
      if (!api || !ex) return;
      const elements = api.getSceneElementsIncludingDeleted();
      const files = api.getFiles();
      const version = ex.getSceneVersion(elements);
      const fileCount = Object.keys(files).length;
      if (version === savedVersion && fileCount === savedFiles) return;
      // Pictures are heavy: sent only when there are new ones.
      const sendFiles = fileCount !== savedFiles ? files : {};
      savedVersion = version;
      savedFiles = fileCount;
      try {
        await invoke("design_save", { id, elements, files: sendFiles });
      } catch (err) {
        store.lastError = String(err);
        savedVersion = -1;
      }
    };

    (async () => {
      try {
        const [mod, react, reactDom, scene] = await Promise.all([
          loadExcalidraw(),
          import("react"),
          import("react-dom/client"),
          invoke<DesignScene>("design_scene", { id }),
        ]);
        if (disposed || !host) return;
        ex = mod;
        const elements = mod.restoreElements(scene.elements as never, null, { refreshDimensions: true, repairBindings: true });
        savedVersion = mod.getSceneVersion(elements);
        savedFiles = Object.keys(scene.files ?? {}).length;
        store.designCounts = meaningCounts(elements as never);
        const initialData = { elements, files: scene.files as never, scrollToContent: true };
        root = reactDom.createRoot(host);
        render = () =>
          root?.render(
            react.createElement(mod.Excalidraw, {
              initialData,
              theme: store.theme === "dk" ? "dark" : "light",
              langCode: i18n.lang === "ko" ? "ko-KR" : "en",
              excalidrawAPI: (a) => {
                api = a;
                store.designApi = a;
              },
              onChange: (els, appState) => {
                const picked = Object.entries(appState.selectedElementIds ?? {})
                  .filter(([, on]) => on)
                  .map(([k]) => k);
                if (picked.join() !== store.designSelected.join()) store.designSelected = picked;
                const version = mod.getSceneVersion(els);
                if (version !== savedVersion || Object.keys(api?.getFiles() ?? {}).length !== savedFiles) {
                  store.designCounts = meaningCounts(els as never);
                  clearTimeout(timer);
                  timer = setTimeout(() => void save(), SAVE_AFTER);
                }
              },
              UIOptions: { canvasActions: { loadScene: false, saveToActiveFile: false, toggleTheme: false } },
            }),
          );
        render();
      } catch (err) {
        failed = String(err);
      }
    })();

    // What the agent drew, folded into what is on screen.
    store.designRemote = (delta: DesignDelta) => {
      if (delta.id !== id || !api || !ex) return;
      const local = api.getSceneElementsIncludingDeleted();
      const remote = ex.restoreElements(delta.elements as never, local, { refreshDimensions: true, repairBindings: true });
      const merged = ex.reconcileElements(local, remote as never, api.getAppState());
      api.updateScene({ elements: merged, captureUpdate: ex.CaptureUpdateAction.IMMEDIATELY });
      // Saved as it now stands, so the measured labels reach the core too.
      clearTimeout(timer);
      timer = setTimeout(() => void save(), SAVE_AFTER);
    };

    return () => {
      disposed = true;
      render = null;
      if (timer) void save();
      store.designRemote = null;
      store.designApi = null;
      // Unmount after the pending save read the scene.
      queueMicrotask(() => root?.unmount());
    };
  });
</script>

<div class="board" bind:this={host}>
  {#if failed}<p class="failed mono">{failed}</p>{/if}
</div>

<style>
  .board {
    position: absolute;
    inset: 0;
  }

  .failed {
    padding: 16px;
    color: var(--deltx);
    font-size: 12px;
  }
</style>
