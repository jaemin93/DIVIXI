<script lang="ts">
  import { store, agentLabel, type Track } from "./store.svelte";
  import { t } from "./i18n.svelte";

  /** The track's name and intent; editing opens the track form, deletion asks twice. */
  let { track }: { track: Track } = $props();

  let confirming = $state(false);

  const index = $derived(store.tracks.findIndex((x) => x.id === track.id) + 1);

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") confirming = false;
  }
</script>

<svelte:window onkeydown={onKey} />

<header>
  <div class="top">
    <span class="mlab">#{String(index).padStart(2, "0")} / TRACK</span>
    <span class="mono meta">{agentLabel(track.agent)}</span>
    <span class="grow"></span>
    {#if confirming}
      <span class="mono note">{store.busy ? t("track.busyNote") : t("track.deleteNote")}</span>
      <button class="btn" type="button" onclick={() => (confirming = false)}>{t("track.cancel")}</button>
      <button class="btn danger" type="button" disabled={store.busy} onclick={() => store.deleteTrack(track.id)}>{t("track.confirmDelete")}</button>
    {:else}
      <button class="btn" type="button" onclick={() => (store.view = "edit-track")}>{t("track.edit")}</button>
      <button class="btn" type="button" onclick={() => (confirming = true)}>{t("track.delete")}</button>
    {/if}
  </div>

  <h1 class="serif">{track.name}</h1>
  <p class:none={!track.intent}>{track.intent || t("track.noIntent")}</p>
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
</style>
