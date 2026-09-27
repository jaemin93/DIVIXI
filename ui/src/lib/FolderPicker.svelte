<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "./ipc.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * Picking a folder on the remote instance being shown (the system picker
   * only knows this PC's): its folders, listed by the instance
   * (`browse_dirs`), walked down and up; the path can be typed too. A new
   * folder is made in the one shown (`make_dir`) and chosen.
   */
  let { start = "", onpick, onclose }: { start?: string; onpick: (path: string) => void; onclose: () => void } = $props();

  type Dirs = { path: string; parent: string | null; home: string; dirs: string[] };

  let at = $state<Dirs | null>(null);
  let typed = $state("");
  let error = $state("");
  let busy = $state(false);
  /** The new folder's name while it is being typed; null when not. */
  let naming = $state<string | null>(null);
  let nameInput = $state<HTMLInputElement>();

  $effect(() => {
    nameInput?.focus();
  });

  async function make(e: Event) {
    e.preventDefault();
    if (!at || naming === null || !naming.trim()) return;
    busy = true;
    error = "";
    try {
      const made = await invoke<string>("make_dir", { parent: at.path, name: naming.trim() });
      naming = null;
      busy = false;
      await go(at.path);
      typed = made;
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function go(path: string | null) {
    busy = true;
    error = "";
    try {
      at = await invoke<Dirs>("browse_dirs", { path });
      typed = at.path;
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }
  // Where it opens: the folder given when it was opened (it does not follow).
  onMount(() => void go(start || null));

  const join = (base: string, name: string) => (base.endsWith("/") || base.endsWith("\\") ? base + name : `${base}${base.includes("\\") ? "\\" : "/"}${name}`);

  function onKey(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    // Escape leaves the name first, then the picker.
    if (naming !== null) naming = null;
    else onclose();
  }

  function onBackdrop(e: MouseEvent) {
    if (e.target === e.currentTarget) onclose();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="backdrop" onclick={onBackdrop} role="presentation">
  <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="fp-title">
    <div class="head">
      <span class="mlab" id="fp-title">{t("folder.title")}</span>
      <span class="grow"></span>
      <button class="x" onclick={onclose} aria-label={t("remote.cancel")}><Icon name="close" size={12} /></button>
    </div>
    <form
      class="path"
      onsubmit={(e) => {
        e.preventDefault();
        void go(typed);
      }}
    >
      <input class="mono" bind:value={typed} spellcheck="false" autocomplete="off" />
      <button class="btn sm" type="submit" disabled={busy}>{t("folder.go")}</button>
    </form>
    <div class="list">
      {#if naming !== null}
        <form class="newrow" onsubmit={make}>
          <Icon name="folder" size={14} />
          <input class="mono" bind:this={nameInput} bind:value={naming} placeholder={t("folder.newName")} spellcheck="false" autocomplete="off" maxlength="255" />
          <button class="btn sm btn-acc" type="submit" disabled={busy || !naming.trim()}>{t("folder.make")}</button>
          <button class="btn sm" type="button" onclick={() => (naming = null)}>{t("remote.cancel")}</button>
        </form>
      {/if}
      {#if at}
        <button class="row" disabled={!at.parent || busy} onclick={() => at?.parent && go(at.parent)}>
          <span class="mono up">..</span>
        </button>
        {#each at.dirs as d (d)}
          <button class="row" disabled={busy} ondblclick={() => at && go(join(at.path, d))} onclick={() => at && (typed = join(at.path, d))}>
            <Icon name="folder" size={14} />
            <span class="mono name">{d}</span>
          </button>
        {:else}
          <div class="empty">{t("folder.empty")}</div>
        {/each}
      {/if}
    </div>
    {#if error}<p class="err mono">{error}</p>{/if}
    <div class="foot">
      <button class="btn sm" disabled={busy || !at} onclick={() => at && go(at.home)}>{t("folder.home")}</button>
      <button class="btn sm" disabled={busy || !at || naming !== null} onclick={() => (naming = "")}>{t("folder.new")}</button>
      <span class="grow"></span>
      <button class="btn" onclick={onclose}>{t("remote.cancel")}</button>
      <button class="btn btn-acc" disabled={busy || !typed.trim()} onclick={() => onpick(typed.trim())}>{t("folder.pick")}</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 32px 0 0 0;
    z-index: 40;
    background: color-mix(in srgb, var(--bg) 60%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }

  .sheet {
    width: min(560px, 100%);
    height: min(560px, 100%);
    display: flex;
    flex-direction: column;
    background: var(--card);
    border: 1px solid var(--lines);
  }

  .head {
    height: 40px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 8px 0 14px;
    border-bottom: 1px solid var(--line);
  }

  .grow {
    flex: 1;
  }

  .x {
    width: 26px;
    height: 26px;
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

  .path {
    display: flex;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--line);
  }

  .path input {
    flex: 1;
    min-width: 0;
    height: 30px;
    padding: 0 9px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-size: 12px;
  }

  .path input:focus {
    border-color: var(--acc);
  }

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 0;
  }

  .row {
    width: 100%;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 14px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--dim);
  }

  .row:hover:not(:disabled) {
    color: var(--hi);
    background: var(--sel);
  }

  .newrow {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    color: var(--dim);
    background: var(--sel);
  }

  .newrow input {
    flex: 1;
    min-width: 0;
    height: 28px;
    padding: 0 8px;
    background: var(--inp);
    border: 1px solid var(--acc);
    color: var(--txt);
    font-size: 12px;
  }

  .name,
  .up {
    font-size: 12px;
  }

  .empty {
    padding: 14px;
    font-size: 12px;
    color: var(--lab);
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid var(--line);
  }

  .btn.sm {
    height: 28px;
    padding: 0 10px;
  }

  .err {
    margin: 0;
    padding: 6px 14px;
    font-size: 11.5px;
    color: var(--deltx);
    white-space: pre-wrap;
  }
</style>
