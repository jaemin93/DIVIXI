<script lang="ts">
  import { store, type Run } from "./store.svelte";
  import SplitHandle from "./SplitHandle.svelte";
  import { t } from "./i18n.svelte";

  let { run }: { run: Run } = $props();
  let tab = $state<"transcript" | "output" | "tools">("transcript");

  const tones: Record<string, string> = {
    in: "var(--body)",
    out: "var(--body)",
    ok: "var(--ok)",
    warn: "var(--warn)",
    dim: "var(--lab)",
  };

</script>

<aside style="width: {store.inspectorWidth}px">
  <SplitHandle
    edge="left"
    width={store.inspectorWidth}
    min={320}
    max={Math.max(360, Math.floor(window.innerWidth * 0.6))}
    reset={430}
    label={t("inspector.width")}
    onchange={(px, persist) => store.setInspectorWidth(px, persist)}
  />
  <div class="head">
    <span class="mlab">LANE / {run.lane}</span>
    <span class="mono id">{run.id}</span>
    <span class="grow"></span>
    <button class="btn x" onclick={() => (store.inspecting = "")} aria-label={t("inspector.close")}>✕</button>
  </div>

  <div class="membrane">
    <div class="mlab-sm acc">{t("inspector.membrane")}</div>
    <div class="note">{t("inspector.membraneNote")}</div>
  </div>

  <div class="tabs">
    {#each [["transcript", "TRANSCRIPT"], ["output", "OUTPUT"], ["tools", "TOOLS"]] as [id, label] (id)}
      <button class="tab" class:on={tab === id} onclick={() => (tab = id as typeof tab)}>{label}</button>
    {/each}
  </div>

  <div class="pane">
    {#if tab === "transcript"}
      {#each run.transcript as line (line.ms + line.label + line.text)}
        <div class="mono ts">{(line.ms / 1000).toFixed(2)}s</div>
        <div class="mono tl" style="color: {tones[line.tone]}">{line.label} · {line.text}</div>
      {/each}
      {#if run.transcript.length === 0}
        <div class="mono ts">{t("inspector.nothingYet")}</div>
      {/if}
    {:else if tab === "output"}
      {#if run.thought}
        <div class="mlab-sm">THOUGHT</div>
        <pre class="dimtext">{run.thought}</pre>
      {/if}
      <div class="mlab-sm">MESSAGE</div>
      <pre>{run.message || t("inspector.none")}</pre>
    {:else}
      {#each run.tools as tool (tool.id)}
        <div class="toolrow mono">
          <span class="tk">{tool.toolKind}</span>
          <span class="tt">{tool.title}</span>
          <span class="tst" class:done={tool.status === "completed"}>{tool.status}</span>
        </div>
      {/each}
      {#if run.tools.length === 0}
        <div class="mono ts">{t("inspector.noTools")}</div>
      {/if}
    {/if}
  </div>
</aside>

<style>
  aside {
    position: relative;
    flex-shrink: 0;
    border-left: 1px solid var(--line);
    background: var(--rail);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }


  .head {
    height: 44px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 16px;
    border-bottom: 1px solid var(--line);
  }

  .id {
    font-size: 11px;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .x {
    width: 28px;
    height: 28px;
    padding: 0;
  }

  .membrane {
    flex-shrink: 0;
    background: var(--memb);
    border-bottom: 1px solid var(--membln);
    padding: 11px 16px;
  }

  .acc {
    color: var(--acct);
    margin-bottom: 5px;
  }

  .note {
    font-size: 11px;
    line-height: 1.55;
    color: var(--dim);
  }

  .tabs {
    flex-shrink: 0;
    display: flex;
    border-bottom: 1px solid var(--line);
  }

  .tab {
    flex: 1;
    height: 38px;
    background: transparent;
    border: 0;
    border-bottom: 1px solid transparent;
    color: var(--dim);
    font-family: var(--mono);
    font-size: 10px;
    letter-spacing: 0.16em;
  }

  .tab.on {
    color: var(--hi);
    border-bottom-color: var(--acc);
  }

  .pane {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px;
  }

  .ts {
    font-size: 11px;
    color: var(--lab);
  }

  .tl {
    font-size: 11px;
    line-height: 1.6;
    margin-bottom: 10px;
    word-break: break-word;
  }

  pre {
    margin: 6px 0 16px;
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1.7;
    color: var(--body);
    white-space: pre-wrap;
    word-break: break-word;
  }

  pre.dimtext {
    color: var(--lab);
  }

  .toolrow {
    display: flex;
    gap: 12px;
    padding: 9px 0;
    border-bottom: 1px solid var(--lineq);
    font-size: 11px;
  }

  .tk {
    width: 64px;
    flex-shrink: 0;
    color: var(--lab);
  }

  .tt {
    flex: 1;
    color: var(--body);
    word-break: break-word;
  }

  .tst {
    color: var(--warn);
  }

  .tst.done {
    color: var(--ok);
  }
</style>
