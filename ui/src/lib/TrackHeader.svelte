<script lang="ts">
  import { store, agentLabel, type Track } from "./store.svelte";
  import PanelToggle from "./PanelToggle.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The track's name, intent and tags. Editing, tagging and deleting live
   * in the track list's context menu; the controls here are the panel and
   * the fold.
   *
   * Folded, the header is one line — name and the first line of the intent,
   * cut with an ellipsis — so a long brief does not eat the conversation.
   * It sits above the scrolling timeline, never inside it, so the fold is
   * in reach however far down the conversation has been read.
   */
  let { track }: { track: Track } = $props();

  const index = $derived(store.tracks.findIndex((x) => x.id === track.id) + 1);
  const folded = $derived(store.isHeaderFolded(track.id));
  /** Folded, only the first line of the intent shows. */
  const gist = $derived(track.intent.split("\n").find((l) => l.trim()) ?? "");

  function toggle() {
    store.setHeaderFolded(track.id, !folded);
  }

  /** Anywhere on the header folds it, except the controls sitting on it. */
  function onHeaderClick(e: MouseEvent) {
    if ((e.target as HTMLElement | null)?.closest("button")) return;
    // Reading the name by selecting it is not a click on the header.
    if (window.getSelection()?.toString()) return;
    toggle();
  }
</script>

<!-- The click on the header itself is a second way to reach the fold; the
     chevron beside it is the real control, focusable and labelled, so the
     keyboard and screen readers have it without a handler here. -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<header class:folded onclick={onHeaderClick}>
  <div class="top">
    {#if track.color}<span class="swatch" style="background: {track.color}"></span>{/if}
    <span class="mlab">#{String(index).padStart(2, "0")} / TRACK</span>
    <span class="mono meta">{agentLabel(track.agent)}</span>
    {#each track.tags as tag (tag)}<span class="mono tag" style="color: {store.tagColor(tag)}; border-color: {store.tagColor(tag)}">{tag}</span>{/each}
    {#if folded}
      <!-- Folded: what the three lines below would have said, in one. -->
      <span class="gist" title={track.intent || track.name}>
        <span class="gname">{track.name}</span>
        {#if gist}<span class="gsep">—</span><span class="gtext">{gist}</span>{/if}
      </span>
    {:else}
      <span class="grow"></span>
    {/if}
    <button
      class="tog chev"
      type="button"
      aria-expanded={!folded}
      title={folded ? t("track.unfoldHeader") : t("track.foldHeader")}
      aria-label={folded ? t("track.unfoldHeader") : t("track.foldHeader")}
      onclick={toggle}
    >
      {folded ? "▸" : "▾"}
    </button>
    <PanelToggle on={store.panelOpen} title={t("ws.toggle")} onclick={() => store.setPanel(!store.panelOpen)} />
  </div>

  {#if !folded}
    <h1 class="serif">{track.name}</h1>
    <p class:none={!track.intent}>{track.intent || t("track.noIntent")}</p>
  {/if}
</header>

<style>
  header {
    padding: 14px 10px 16px 34px;
    border-bottom: 1px solid var(--line);
    flex-shrink: 0;
    cursor: pointer;
  }

  /* Folded it is one row of controls: the padding comes in to match. */
  header.folded {
    padding: 8px 10px 8px 34px;
  }

  .top {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 28px;
  }

  .swatch {
    width: 3px;
    height: 16px;
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
    border: 1px solid;
    padding: 1px 6px;
  }

  .grow {
    flex: 1;
  }

  /* The one-line summary takes the space the spacer would: it is what
     gives way when the row is tight. */
  .gist {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 7px;
    overflow: hidden;
    white-space: nowrap;
  }

  .gname {
    font-family: var(--serif);
    font-size: 15px;
    color: var(--hi);
    flex-shrink: 0;
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .gsep {
    color: var(--lab);
    flex-shrink: 0;
  }

  .gtext {
    font-size: 12px;
    color: var(--dim);
    overflow: hidden;
    text-overflow: ellipsis;
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
    flex-shrink: 0;
  }

  .tog:hover {
    color: var(--hi);
    background: var(--sel);
  }

  /* The fold is a quieter control than the panel: no box around it. */
  .chev {
    border-color: transparent;
    font-size: 12px;
    line-height: 1;
    color: var(--lab);
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
