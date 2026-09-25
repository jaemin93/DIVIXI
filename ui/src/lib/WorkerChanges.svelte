<script lang="ts">
  import { store, type WorkerChanges, type WorkerMerged } from "./store.svelte";
  import { highlight } from "./highlight";
  import { t } from "./i18n.svelte";

  /**
   * What a worker changed in its own checkout and the human has not brought
   * into the track folder: the files, each diff on demand, and merge or
   * discard. Merging is all or nothing; a conflict writes nothing.
   */
  let { track, worker }: { track: string; worker: string } = $props();

  const key = $derived(`${track}/${worker}`);
  const changes = $derived<WorkerChanges | undefined>(store.workerChanges[key]);
  let openPath = $state("");
  let diffs = $state<Record<string, string>>({});
  let busy = $state(false);
  let confirming = $state(false);
  let outcome = $state<WorkerMerged | null>(null);
  let error = $state("");

  $effect(() => {
    // Asked again when a report arrives: the worker may have changed more.
    void store.reportsArrived;
    void store.loadWorkerChanges(track, worker);
  });

  /** A path as the track folder sees it (the checkout's paths start at the repository's top). */
  function shown(path: string): string {
    const sub = changes?.sub ?? "";
    return sub && path.startsWith(`${sub}/`) ? path.slice(sub.length + 1) : path;
  }

  async function toggle(path: string) {
    if (openPath === path) {
      openPath = "";
      return;
    }
    openPath = path;
    if (diffs[path] === undefined) {
      const text = await store.workerFileDiff(track, worker, path);
      diffs = { ...diffs, [path]: text };
    }
  }

  async function merge() {
    busy = true;
    error = "";
    try {
      outcome = await store.mergeWorker(track, worker);
      if (outcome.files.length) diffs = {};
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function discard() {
    busy = true;
    error = "";
    try {
      await store.discardWorker(track, worker);
      outcome = null;
      diffs = {};
      openPath = "";
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
      confirming = false;
    }
  }
</script>

{#if changes?.isolated && (changes.files.length || outcome)}
  <div class="wc">
    {#if changes.files.length}
      <div class="whead">
        <span class="mlab-sm">{t("wc.pending", { n: changes.files.length })}</span>
        <span class="mono branch" title={t("wc.branchTitle")}>{changes.branch}</span>
      </div>
      <ul>
        {#each changes.files as f (f.path)}
          <li>
            <button type="button" class="frow" class:on={openPath === f.path} onclick={() => toggle(f.path)} aria-expanded={openPath === f.path}>
              <span class="mono st s-{f.status}">{f.status}</span>
              <span class="mono path">{shown(f.path)}</span>
              <span class="grow"></span>
              {#if f.added !== null && f.removed !== null}
                <span class="mono add">+{f.added}</span>
                <span class="mono del">−{f.removed}</span>
              {:else}
                <span class="mono bin">{t("wc.binary")}</span>
              {/if}
            </button>
            {#if openPath === f.path}
              <pre class="diff hljs">{@html highlight(diffs[f.path] ?? "", "diff")}</pre>
            {/if}
          </li>
        {/each}
      </ul>
      {#if outcome?.conflicts.length}
        <p class="warn">{t("wc.conflicts", { files: outcome.conflicts.map(shown).join(", ") })}</p>
      {/if}
      {#if error}<p class="warn">{error}</p>{/if}
      <div class="actions">
        {#if confirming}
          <span class="ask">{t("wc.discardAsk")}</span>
          <button type="button" class="btn sm danger" disabled={busy} onclick={discard}>{t("wc.discard")}</button>
          <button type="button" class="btn sm" disabled={busy} onclick={() => (confirming = false)}>{t("wc.cancel")}</button>
        {:else}
          <button type="button" class="btn btn-acc sm" disabled={busy} onclick={merge}>{busy ? t("wc.merging") : t("wc.merge")}</button>
          <button type="button" class="btn sm" disabled={busy} onclick={() => (confirming = true)}>{t("wc.discard")}</button>
        {/if}
      </div>
    {:else if outcome?.files.length}
      <p class="done mono">✓ {t("wc.merged", { n: outcome.files.length })}</p>
    {/if}
  </div>
{/if}

<style>
  .wc {
    margin-top: 10px;
    padding-top: 9px;
    border-top: 1px solid var(--line);
  }

  .whead {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .whead .mlab-sm {
    color: var(--warn);
  }

  .branch {
    font-size: 10px;
    color: var(--lab);
  }

  ul {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
  }

  .frow {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 4px;
    background: none;
    border: none;
    text-align: left;
    cursor: pointer;
    color: var(--txt);
  }

  .frow:hover,
  .frow.on {
    background: var(--sel);
  }

  .st {
    width: 12px;
    font-size: 10.5px;
    text-align: center;
  }

  .s-A {
    color: var(--ok);
  }

  .s-M {
    color: var(--warn);
  }

  .s-D {
    color: var(--deltx);
  }

  .path {
    font-size: 11.5px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .grow {
    flex: 1;
  }

  .add {
    font-size: 10.5px;
    color: var(--ok);
  }

  .del,
  .bin {
    font-size: 10.5px;
    color: var(--deltx);
  }

  .bin {
    color: var(--lab);
  }

  .diff {
    margin: 4px 0 8px;
    max-height: 320px;
    overflow: auto;
    padding: 8px 10px;
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1.5;
    background: var(--inp);
    border: 1px solid var(--line);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 8px;
  }

  .ask {
    font-size: 12px;
    color: var(--warn);
  }

  .btn.sm {
    height: 26px;
    padding: 0 12px;
  }

  .btn.danger {
    color: var(--deltx);
    border-color: var(--deltx);
  }

  .warn {
    margin: 8px 0 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--warn);
  }

  .done {
    margin: 0;
    font-size: 11px;
    color: var(--ok);
  }
</style>
