<script lang="ts">
  import { store, agentLabel, type Track } from "./store.svelte";
  import { t } from "./i18n.svelte";

  /** The track's name and intent, editable in place; deletion asks twice. */
  let { track }: { track: Track } = $props();

  let editing = $state(false);
  let name = $state("");
  let intent = $state("");
  let confirming = $state(false);

  const index = $derived(store.tracks.findIndex((x) => x.id === track.id) + 1);

  function edit() {
    name = track.name;
    intent = track.intent;
    editing = true;
    confirming = false;
  }

  async function save(e: Event) {
    e.preventDefault();
    if (!name.trim()) return;
    await store.updateTrack(track.id, { name: name.trim(), intent: intent.trim() });
    editing = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      editing = false;
      confirming = false;
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<header>
  <div class="top">
    <span class="mlab">#{String(index).padStart(2, "0")} / TRACK</span>
    <span class="mono meta">{agentLabel(track.agent)}</span>
    <span class="grow"></span>
    {#if editing}
      <button class="btn" type="button" onclick={() => (editing = false)}>{t("track.cancel")}</button>
      <button class="btn btn-acc" type="submit" form="track-edit" disabled={!name.trim()}>{t("track.save")}</button>
    {:else if confirming}
      <span class="mono note">{store.busy ? t("track.busyNote") : t("track.deleteNote")}</span>
      <button class="btn" type="button" onclick={() => (confirming = false)}>{t("track.cancel")}</button>
      <button class="btn danger" type="button" disabled={store.busy} onclick={() => store.deleteTrack(track.id)}>{t("track.confirmDelete")}</button>
    {:else}
      <button class="btn" type="button" onclick={edit}>{t("track.edit")}</button>
      <button class="btn" type="button" onclick={() => (confirming = true)}>{t("track.delete")}</button>
    {/if}
  </div>

  {#if editing}
    <form id="track-edit" onsubmit={save}>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="serif name" type="text" bind:value={name} maxlength="80" autofocus aria-label={t("newtrack.name")} />
      <input class="intent" type="text" bind:value={intent} maxlength="200" placeholder={t("newtrack.intentPh")} aria-label={t("newtrack.intent")} />
    </form>
  {:else}
    <h1 class="serif">{track.name}</h1>
    <p class:none={!track.intent}>{track.intent || t("track.noIntent")}</p>
  {/if}
</header>

<style>
  header {
    padding: 22px 34px 16px;
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
  }

  .top {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 28px;
  }

  .meta {
    font-size: 10px;
    letter-spacing: 0.12em;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .note {
    font-size: 11px;
    color: var(--dim);
  }

  .btn.danger:not(:disabled) {
    color: var(--acct);
    border-color: var(--acct);
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

  p.none {
    color: var(--lab);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
    max-width: 620px;
  }

  input {
    background: var(--inp);
    border: 1px solid var(--line);
    color: var(--txt);
    outline: none;
    padding: 4px 10px;
  }

  input:focus {
    border-color: var(--acc);
  }

  input.name {
    font-size: 30px;
    line-height: 1.1;
    color: var(--hi);
  }

  input.intent {
    font-family: var(--sans);
    font-size: 13px;
    line-height: 1.6;
  }
</style>
