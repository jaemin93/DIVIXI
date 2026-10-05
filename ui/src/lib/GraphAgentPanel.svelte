<script lang="ts">
  import { store } from "./store.svelte";
  import type { Chip } from "./graphContext";
  import SplitHandle from "./SplitHandle.svelte";
  import Timeline from "./Timeline.svelte";
  import Composer from "./Composer.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The knowledge graph's conversation, as a column on the right of the
   * knowledge page — where a design's conversation sits beside its board:
   * the same Timeline and Composer, on the graph's own artifact. Above them,
   * what goes with the next message (store.graphChips, set by the graph
   * view), as chips to take out and put back.
   */

  function chipLabel(c: Chip): string {
    switch (c.kind) {
      case "entity":
        return t("kb.agentChipEntity", { name: c.name });
      case "sources":
        return t("kb.agentChipSources", { n: c.count, name: c.name });
      case "focus":
        return t("kb.agentChipFocus", { n: c.count, name: c.name });
      default:
        return t("kb.agentChipQuery", { n: c.count, name: c.name });
    }
  }
</script>

<aside class="talk" style="width: {store.graphChatWidth}px" aria-label={t("kb.agentLabel")}>
  <SplitHandle
    edge="left"
    width={store.graphChatWidth}
    min={320}
    max={640}
    reset={400}
    label={t("kb.agentResize")}
    onchange={(px) => store.setGraphChatWidth(px)}
  />
  <div class="head">
    <span class="mono hlab">{t("kb.agentLabel")}</span>
    <span class="grow"></span>
    <button class="x" onclick={() => store.closeGraphChat()} aria-label={t("kb.agentClose")} title={t("kb.agentClose")}>×</button>
  </div>
  <div class="ctx">
    <div class="mono ctxlab">{t("kb.agentContext")}</div>
    {#if store.graphChips.length}
      <div class="chips">
        {#each store.graphChips as c (c.key)}
          {@const out = store.graphExcluded.has(c.key)}
          <button
            class="kchip"
            class:out
            aria-pressed={!out}
            title={out ? t("kb.agentChipAdd") : t("kb.agentChipRemove")}
            onclick={() => store.toggleGraphChip(c.key)}
          >
            <span class="clabel">{chipLabel(c)}</span>
            <span class="cx" aria-hidden="true">{out ? "+" : "×"}</span>
          </button>
        {/each}
      </div>
    {:else}
      <p class="ctxnone">{t("kb.agentNothing")}</p>
    {/if}
  </div>
  <Timeline />
  <Composer />
</aside>

<style>
  /* As a design's conversation column (DesignView .talk). */
  .talk {
    position: relative;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 1px solid var(--line);
    background-image: radial-gradient(var(--dot) 1px, transparent 1px);
    background-size: 22px 22px;
  }

  .talk :global(.composer) {
    padding-left: 16px;
    padding-right: 16px;
  }

  .talk :global(.scroll) {
    padding-left: 18px;
    padding-right: 18px;
  }

  .head {
    display: flex;
    align-items: center;
    height: 44px;
    padding: 0 10px 0 16px;
    border-bottom: 1px solid var(--line);
    background: var(--bg);
  }

  .hlab {
    font-size: 10px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--dim);
  }

  .grow {
    flex: 1;
  }

  .x {
    background: transparent;
    border: 0;
    color: var(--lab);
    font-size: 16px;
  }

  .x:hover {
    color: var(--hi);
  }

  /* What goes with the next message. */
  .ctx {
    padding: 10px 16px 10px;
    border-bottom: 1px solid var(--line);
    background: var(--bg);
  }

  .ctxlab {
    font-size: 9px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--kg-acc);
    margin-bottom: 6px;
  }

  .ctxnone {
    margin: 0;
    font-size: 12px;
    color: var(--lab);
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .kchip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    height: 22px;
    padding: 0 6px 0 8px;
    background: transparent;
    border: 1px solid var(--kg-acc);
    color: var(--txt);
    font-size: 11.5px;
  }

  .kchip .clabel {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kchip .cx {
    color: var(--lab);
  }

  .kchip:hover .cx {
    color: var(--hi);
  }

  .kchip.out {
    border-style: dashed;
    border-color: var(--lines);
    color: var(--lab);
    text-decoration: line-through;
  }
</style>
