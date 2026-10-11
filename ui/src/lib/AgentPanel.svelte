<script lang="ts">
  import { store } from "./store.svelte";
  import { kb } from "./knowledge.svelte";
  import { plans } from "./plan.svelte";
  import type { Chip } from "./graphContext";
  import SplitHandle from "./SplitHandle.svelte";
  import Timeline from "./Timeline.svelte";
  import Composer from "./Composer.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The agent beside a page, as a column of the app's layout on its right —
   * the same place, toggle and shape on the designs page and the knowledge
   * page (and where a track has its working folder). The same Timeline and
   * Composer as a track's, on the page's own artifact: the open design, or
   * the knowledge graph's conversation. For the graph, what goes with the
   * next message (store.graphChips) sits above it as chips to take out and
   * put back; a design sends its board and the items picked on it. A
   * routine's plan sends the routine, the plan and the steps picked on it;
   * its agent is its own (the composer's picker below sets it for the plan),
   * never the routine's, which runs the work.
   */
  let { kind }: { kind: "design" | "graph" | "plan" } = $props();
  /** The design's and the plan's panel share a width: both sit beside a page of their own. */
  const design = $derived(kind === "design" || kind === "plan");
  const label = $derived(kind === "plan" ? t("plan.agentLabel") : kind === "design" ? t("design.agentLabel") : t("kb.agentLabel"));
  const planSteps = $derived(kind === "plan" ? plans.doc.steps.filter((s) => store.planSelected.includes(s.id)) : []);

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

<aside
  class="talk"
  style="width: {design ? store.artifactChatWidth : store.graphChatWidth}px"
  aria-label={label}
>
  {#if design}
    <SplitHandle
      edge="left"
      width={store.artifactChatWidth}
      min={340}
      max={760}
      reset={460}
      label={t("design.chatResize")}
      onchange={(px, persist) => store.setArtifactChatWidth(px, persist)}
    />
  {:else}
    <SplitHandle
      edge="left"
      width={store.graphChatWidth}
      min={320}
      max={640}
      reset={400}
      label={t("kb.agentResize")}
      onchange={(px) => store.setGraphChatWidth(px)}
    />
  {/if}
  <div class="head">
    <span class="mono hlab">{label}</span>
    <span class="grow"></span>
    <button
      class="x"
      onclick={() => (kind === "plan" ? store.setPlanChat(false) : design ? store.setDesignChat(false) : store.closeGraphChat())}
      aria-label={t("panel.agentClose")}
      title={t("panel.agentClose")}>×</button
    >
  </div>
  {#if kind === "plan"}
  <!-- The steps picked on the plan go with the next message; nothing is said
       about it when none are. -->
  {#if planSteps.length}
    <div class="ctx">
      <div class="chips">
        {#each planSteps as s (s.id)}
          <button class="kchip" title={t("kb.agentChipRemove")} onclick={() => plans.pick(s.id, true)}>
            <span class="clabel">{t("plan.agentChip", { id: s.id, title: s.title })}</span>
            <span class="cx" aria-hidden="true">×</span>
          </button>
        {/each}
      </div>
    </div>
  {/if}
  {:else if !design}
  <!-- What goes with the next message: the graph's chips, to take out and put back. -->
  {#if store.graphChips.length}
    <div class="ctx">
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
    </div>
  {/if}
  {/if}
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

  /* A phone (Track's breakpoint): no room beside the board or the knowledge
     page, so the agent covers the screen, as a track's working folder does
     (Workspace): the strip above the soft keyboard. Its own × (or the page's
     panel toggle, under it) takes it back to the page. No width to drag. */
  @media (max-width: 640px) {
    .talk {
      position: fixed;
      left: 0;
      right: 0;
      top: var(--vvtop, 0px);
      height: var(--vvh, 100dvh);
      width: auto !important;
      z-index: 30;
      border-left: 0;
      background-color: var(--bg);
    }

    .talk > :global([role="separator"]) {
      display: none;
    }

    .talk .x {
      width: 44px;
      height: 44px;
    }
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
