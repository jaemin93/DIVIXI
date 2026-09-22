<script lang="ts">
  import { store } from "./store.svelte";

  // Phase 0 runs everything in one lane; the rail shows the shape the
  // domain model already has, not more than the core can back.
  const lanes = $derived([
    {
      name: "solo",
      status: store.activeRun ? "running" : store.runs.length ? "idle" : "unspawned",
      runs: store.runs.length,
    },
  ]);
</script>

<nav>
  <div class="mlab pad">Tracks</div>
  <button class="row active">
    <span class="dot" style="background: var(--ok)"></span>
    <span class="name">ACP 브리지</span>
    <span class="mono count">{store.runs.length}</span>
  </button>

  <div class="mlab pad top">Lanes</div>
  {#each lanes as lane (lane.name)}
    <div class="row static">
      <span
        class="dot"
        class:pulse={lane.status === "running"}
        style="background: {lane.status === 'running' ? 'var(--ok)' : 'var(--idle)'}"
      ></span>
      <span class="name mono">{lane.name}</span>
      <span class="mono count">{lane.runs}</span>
    </div>
  {/each}

  <span class="grow"></span>

  <div class="mlab pad">Agents</div>
  <div class="agent mono">
    <span class="dot" style="background: var(--ok)"></span>claude-code<span class="grow"></span>
    <span class="count">acp</span>
  </div>
</nav>

<style>
  nav {
    width: 224px;
    flex-shrink: 0;
    border-right: 1px solid var(--line);
    background: var(--rail);
    display: flex;
    flex-direction: column;
    padding: 18px 0 14px;
  }

  .pad {
    padding: 0 16px 9px;
  }

  .top {
    padding-top: 24px;
  }

  .row {
    width: 100%;
    height: 40px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 16px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    text-align: left;
    font-size: 13px;
    color: var(--dim);
  }

  .row.static {
    cursor: default;
  }

  .row.active {
    background: var(--sel);
    border-left-color: var(--acc);
    color: var(--hi);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .name {
    flex: 1;
    font-size: 13px;
  }

  .count {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .agent {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 16px;
    font-size: 11px;
    color: var(--dim);
  }
</style>
