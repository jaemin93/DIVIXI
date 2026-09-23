<script lang="ts">
  import { store } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * Tagging a track, in the middle of the screen as Kiro Crew does it: the
   * tags the user has made so far (every tag on any track), each toggling
   * on this track, and a line to make a new one. No presets.
   */
  let draft = $state("");
  let input = $state<HTMLInputElement>();

  const track = $derived(store.tracks.find((x) => x.id === store.tagDialog));
  const pool = $derived(store.allTags);

  $effect(() => {
    input?.focus();
  });

  function close() {
    store.tagDialog = "";
  }

  async function toggle(tag: string) {
    if (!track) return;
    const has = track.tags.includes(tag);
    await store.updateTrack(track.id, { tags: has ? track.tags.filter((x) => x !== tag) : [...track.tags, tag] });
  }

  async function add(e: Event) {
    e.preventDefault();
    const tag = draft.trim();
    if (!track || !tag) return;
    draft = "";
    if (!track.tags.includes(tag)) await store.updateTrack(track.id, { tags: [...track.tags, tag] });
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") close();
  }

  function onBackdrop(e: MouseEvent) {
    if (e.target === e.currentTarget) close();
  }
</script>

<svelte:window onkeydown={onKey} />

{#if track}
  <div class="backdrop" onclick={onBackdrop} role="presentation">
    <div class="sheet" role="dialog" aria-modal="true" aria-labelledby="tag-title">
      <div class="head">
        <span class="mlab" id="tag-title">{t("track.tagTitle")}</span>
        <span class="mono dim">{track.name}</span>
        <span class="grow"></span>
        <button class="x" onclick={close} aria-label={t("track.cancel")}><Icon name="close" size={12} /></button>
      </div>

      <div class="list">
        {#each pool as tag (tag)}
          {@const on = track.tags.includes(tag)}
          <button class="row" class:on role="checkbox" aria-checked={on} onclick={() => toggle(tag)}>
            <span class="box" class:on></span>
            <span class="mono name">{tag}</span>
            <span class="grow"></span>
            <span class="mono count">{store.tracks.filter((x) => x.tags.includes(tag)).length}</span>
          </button>
        {/each}
        {#if pool.length === 0}
          <div class="mono empty">{t("track.noTags")}</div>
        {/if}
      </div>

      <form class="new" onsubmit={add}>
        <input type="text" bind:this={input} bind:value={draft} placeholder={t("track.newTag")} maxlength="24" spellcheck="false" />
        <button class="btn" type="submit" disabled={!draft.trim()}>{t("track.addTag")}</button>
      </form>
    </div>
  </div>
{/if}

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
    width: min(360px, 100%);
    max-height: min(520px, 100%);
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
    gap: 10px;
    padding: 0 8px 0 14px;
    border-bottom: 1px solid var(--line);
  }

  .dim {
    font-size: 10px;
    color: var(--lab);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px 0;
  }

  .row {
    width: 100%;
    height: 32px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 14px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--dim);
  }

  .row:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .row.on {
    color: var(--hi);
  }

  /* A square box; filled when the tag is on this track. */
  .box {
    width: 10px;
    height: 10px;
    flex-shrink: 0;
    border: 1px solid var(--lines);
  }

  .box.on {
    background: var(--acc);
    border-color: var(--acc);
  }

  .name {
    font-size: 12px;
  }

  .count {
    font-size: 9px;
    color: var(--lab);
  }

  .empty {
    padding: 14px;
    font-size: 11px;
    color: var(--lab);
  }

  .new {
    flex-shrink: 0;
    display: flex;
    gap: 8px;
    padding: 10px 14px 12px;
    border-top: 1px solid var(--line);
  }

  .new input {
    flex: 1;
    min-width: 0;
    height: 30px;
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 12px;
    padding: 0 9px;
    outline: none;
  }

  .new input:focus {
    border-color: var(--acc);
  }
</style>
