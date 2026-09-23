<script lang="ts">
  import { store } from "./store.svelte";
  import AgentPicker from "./AgentPicker.svelte";
  import Icon from "./Icon.svelte";
  import Popover from "./Popover.svelte";
  import { t } from "./i18n.svelte";

  let draft = $state("");
  let contextOpen = $state(false);
  let box = $state<HTMLTextAreaElement>();
  /** Highlighted row in the slash list. */
  let slashIndex = $state(0);

  function submit(e?: Event) {
    e?.preventDefault();
    const text = draft;
    if (!text.trim() || store.busy) return;
    draft = "";
    store.send(text);
    queueMicrotask(grow);
  }

  /** The box grows with its text, up to eight lines; only past that does it scroll. */
  const MAX_BOX = 8 * 22 + 24;
  function grow() {
    if (!box) return;
    box.style.height = "auto";
    const wanted = box.scrollHeight;
    box.style.height = `${Math.min(wanted, MAX_BOX)}px`;
    box.style.overflowY = wanted > MAX_BOX ? "auto" : "hidden";
  }

  // ----- slash commands -----
  /** The word being typed after a leading "/", or null when not completing. */
  const slashQuery = $derived.by(() => {
    if (!draft.startsWith("/") || draft.includes("\n")) return null;
    const m = draft.match(/^\/([^\s]*)$/);
    return m ? m[1].toLowerCase() : null;
  });
  const slashMatches = $derived(
    slashQuery === null ? [] : store.slashCommands.filter((c) => c.name.toLowerCase().startsWith(slashQuery)),
  );
  const slashOpen = $derived(slashQuery !== null && slashMatches.length > 0);

  // Typing "/" is intent to talk to the conductor: with no list known yet,
  // open its session now so the commands (and the first reply) are ready.
  $effect(() => {
    if (slashQuery !== null && store.slashCommands.length === 0 && !store.conductorState.open) void store.openConductor();
  });
  const slashWaiting = $derived(slashQuery !== null && slashMatches.length === 0 && store.conductorOpening === store.track);

  $effect(() => {
    void slashMatches.length;
    slashIndex = 0;
  });

  /** Put a command in the box; one without input goes straight out. */
  function complete(name: string) {
    const cmd = store.slashCommands.find((c) => c.name === name);
    draft = `/${name} `;
    if (cmd && !cmd.hint) {
      submit();
      return;
    }
    box?.focus();
    queueMicrotask(grow);
  }

  function onKey(e: KeyboardEvent) {
    if (slashOpen) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        slashIndex = (slashIndex + 1) % slashMatches.length;
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        slashIndex = (slashIndex - 1 + slashMatches.length) % slashMatches.length;
        return;
      }
      if (e.key === "Tab" || e.key === "Enter") {
        e.preventDefault();
        complete(slashMatches[slashIndex].name);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        draft = "";
        return;
      }
    }
    // Enter sends; Shift+Enter (and Ctrl/Alt+Enter) breaks the line.
    if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.isComposing) {
      e.preventDefault();
      submit();
    }
  }

  const ctx = $derived(store.context);
  const pct = $derived(ctx && ctx.size > 0 ? Math.min(100, (ctx.used / ctx.size) * 100) : 0);

  function k(n: number): string {
    return n >= 1000 ? `${(n / 1000).toFixed(n >= 100000 ? 0 : 1)}k` : String(n);
  }

  function shortPath(p: string | undefined): string {
    if (!p) return "";
    const parts = p.split(/[\\/]/).filter(Boolean);
    if (parts.length <= 3) return p;
    const sep = p.includes("\\") ? "\\" : "/";
    return `…${sep}${parts.slice(-2).join(sep)}`;
  }

</script>

<div class="composer">
  <form onsubmit={submit}>
    <div class="boxwrap">
      {#if slashWaiting}
        <div class="slash" role="status">
          <div class="mono waiting"><span class="dot pulse"></span>{t("composer.opening")}</div>
        </div>
      {:else if slashOpen}
        <!-- The agent's slash commands, narrowed by what follows the "/". -->
        <div class="slash" role="listbox" aria-label={t("composer.commands")}>
          <div class="mlab-sm ph">{t("composer.commands")}</div>
          {#each slashMatches as c, i (c.name)}
            <button
              type="button"
              class="cmd"
              class:on={i === slashIndex}
              role="option"
              aria-selected={i === slashIndex}
              onmouseenter={() => (slashIndex = i)}
              onclick={() => complete(c.name)}
            >
              <span class="mono cname">/{c.name}</span>
              {#if c.hint}<span class="mono chint">{c.hint}</span>{/if}
              <span class="cdesc">{c.description}</span>
            </button>
          {/each}
        </div>
      {/if}
      <textarea
        bind:this={box}
        bind:value={draft}
        rows="1"
        disabled={store.busy}
        placeholder={store.busy ? t("composer.busy") : t("composer.placeholder")}
        aria-label={t("composer.placeholder")}
        onkeydown={onKey}
        oninput={grow}
      ></textarea>
    </div>
    {#if store.busy}
      <!-- While the conductor answers, the send button is a stop button: Ctrl+C. -->
      <button class="btn send stop" type="button" disabled={store.cancelling} onclick={() => store.cancelConductor()} title={t("composer.stopTitle")} aria-label={t("composer.stop")}>■</button>
    {:else}
      <button class="btn send" type="submit" disabled={!draft.trim()} aria-label={t("composer.send")}>→</button>
    {/if}
  </form>

  <div class="status">
    <AgentPicker />

    <span class="chip static" title={store.currentTrack?.cwd}>
      <Icon name="folder" size={14} />
      <span class="mono path">{shortPath(store.currentTrack?.cwd)}</span>
    </span>

    <span class="grow"></span>

    {#if store.lastError}<span class="mono err" title={store.lastError}>{store.lastError}</span>{/if}

    <!-- context -->
    <Popover bind:open={contextOpen} align="right" width={280}>
      {#snippet trigger()}
        <button class="ctx" onclick={() => (contextOpen = !contextOpen)} title={t("composer.contextTitle")} aria-expanded={contextOpen}>
          <span class="bar"><span class="fill" style="width: {pct}%"></span></span>
        </button>
      {/snippet}
      <div class="mlab ph">{t("composer.context")}</div>
      <div class="kv mono">
        {#if ctx}
          <div class="row"><span class="dim">{t("composer.used")}</span><span>{t("composer.tokens", { used: k(ctx.used), size: k(ctx.size) })}</span></div>
          <div class="row"><span class="dim">{t("composer.ratio")}</span><span>{pct.toFixed(1)}%</span></div>
          {#if ctx.cost !== undefined && ctx.cost !== null}
            <div class="row"><span class="dim">{t("composer.cost")}</span><span>{ctx.cost.toFixed(4)} {ctx.currency ?? ""}</span></div>
          {/if}
          <div class="row"><span class="dim">{t("composer.source")}</span><span>{store.activeRun ? t("composer.liveRun") : t("composer.lastRun")}</span></div>
        {:else}
          <div class="row dim">{t("composer.noRuns")}</div>
        {/if}
      </div>
    </Popover>
  </div>
</div>

<style>
  .composer {
    flex-shrink: 0;
    border-top: 1px solid var(--line);
    background: var(--rail);
    padding: 16px 34px 10px;
  }

  form {
    display: flex;
    align-items: flex-end;
    gap: 9px;
  }

  .boxwrap {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
  }

  textarea {
    flex: 1;
    field-sizing: content;
    min-height: 44px;
    max-height: 200px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 14px;
    line-height: 22px;
    padding: 11px 14px;
    outline: none;
    resize: none;
    overflow-y: hidden;
  }

  textarea:focus {
    border-color: var(--acc);
  }

  /* The slash list, above the box. */
  .slash {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 6px);
    z-index: 30;
    max-height: 280px;
    overflow-y: auto;
    background: var(--card);
    border: 1px solid var(--lines);
    padding-bottom: 4px;
  }

  .slash .ph {
    padding: 10px 14px 6px;
  }

  .waiting {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    font-size: 11px;
    color: var(--lab);
  }

  .waiting .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
  }

  .cmd {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 7px 14px;
    background: transparent;
    border: 0;
    border-left: 2px solid transparent;
    text-align: left;
    color: var(--dim);
  }

  .cmd.on {
    color: var(--hi);
    background: var(--sel);
    border-left-color: var(--acc);
  }

  .cname {
    font-size: 12px;
    color: inherit;
    flex-shrink: 0;
  }

  .chint {
    font-size: 10px;
    color: var(--lab);
    flex-shrink: 0;
  }

  .cdesc {
    font-size: 11px;
    color: var(--lab);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }


  .send {
    width: 44px;
    height: 44px;
    padding: 0;
    font-size: 15px;
    letter-spacing: 0;
    border-color: var(--accln);
    color: var(--acct);
  }

  .send.stop {
    font-size: 11px;
    border-color: var(--acct);
  }

  /* One row, always: chips shrink and cut before anything wraps. No
     overflow clipping here: the popovers open upward out of this row. */
  .status {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 32px;
    margin-top: 6px;
    min-width: 0;
    white-space: nowrap;
  }

  .chip {
    height: 26px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 0 8px;
    background: transparent;
    border: 0;
    color: var(--dim);
    font-size: 10px;
    letter-spacing: 0.12em;
  }

  .chip:not(.static):hover {
    color: var(--hi);
    background: var(--sel);
  }

  .chip.static {
    color: var(--lab);
    min-width: 0;
    flex-shrink: 1;
  }

  .path {
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .grow {
    flex: 1;
  }

  .err {
    font-size: 10px;
    color: var(--acct);
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Context: a hairline track, accent fill; no numbers until asked. */
  .ctx {
    height: 26px;
    display: flex;
    align-items: center;
    padding: 0 8px;
    background: transparent;
    border: 0;
  }

  .ctx:hover {
    background: var(--sel);
  }

  .bar {
    width: 72px;
    height: 3px;
    background: var(--lines);
    display: block;
  }

  .fill {
    display: block;
    height: 100%;
    background: var(--acc);
    transition: width 200ms linear;
  }

  .ph {
    padding: 12px 14px 8px;
  }

  .kv {
    display: flex;
    flex-direction: column;
    padding: 0 14px 12px;
    gap: 6px;
    font-size: 11px;
    color: var(--txt);
  }

  .row {
    display: flex;
    gap: 12px;
  }

  .row .dim {
    color: var(--lab);
    width: 40px;
    flex-shrink: 0;
    font-size: 11px;
  }
</style>
