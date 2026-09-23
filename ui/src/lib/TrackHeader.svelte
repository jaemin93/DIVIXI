<script lang="ts">
  import { store, agentLabel, type Track } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The track's name, intent and tags. Editing, tagging and deleting live
   * in the track list's context menu; the only control here is the panel.
   */
  let { track }: { track: Track } = $props();

  const index = $derived(store.tracks.findIndex((x) => x.id === track.id) + 1);
</script>

<header>
  <div class="top">
    {#if track.color}<span class="swatch" style="background: {track.color}"></span>{/if}
    <span class="mlab">#{String(index).padStart(2, "0")} / TRACK</span>
    <span class="mono meta">{agentLabel(track.agent)}</span>
    {#each track.tags as tag (tag)}<span class="mono tag">{tag}</span>{/each}
    <span class="grow"></span>
    <button class="tog" class:on={store.panelOpen} type="button" title={t("ws.toggle")} aria-pressed={store.panelOpen} onclick={() => store.setPanel(!store.panelOpen)}>
      <Icon name="panel" size={14} />
    </button>
  </div>

  <h1 class="serif">{track.name}</h1>
  <p class:none={!track.intent}>{track.intent || t("track.noIntent")}</p>
</header>

<style>
  header {
    padding: 14px 10px 16px 34px;
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
  }

  .top {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 28px;
  }

  .swatch {
    width: 8px;
    height: 8px;
    flex-shrink: 0;
  }

  .meta {
    font-size: 10px;
    letter-spacing: 0.12em;
    color: var(--lab);
  }

  .tag {
    font-size: 9px;
    letter-spacing: 0.1em;
    color: var(--dim);
    border: 1px solid var(--line);
    padding: 1px 6px;
  }

  .grow {
    flex: 1;
  }

  /* The panel toggle hugs the edge, where Kiro keeps its panel button. */
  .tog {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--dim);
  }

  .tog:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .tog.on {
    color: var(--hi);
    border-color: var(--acc);
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
