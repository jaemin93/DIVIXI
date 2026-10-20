<script lang="ts">
  import { untrack } from "svelte";
  import { store, type Watched } from "./store.svelte";
  import Popover from "./Popover.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";
  import { whenFull } from "./time";
  import { isCondition, lateness, message, nextLabel, rule, upcoming } from "./scheduleList";

  /**
   * The track's schedules, one row each, beside the panel toggle at the top
   * right: what will wake the conductor and when, soonest first. Shown only
   * while the track has one. Other tracks' follow under their own heading,
   * read when the popover opens.
   *
   * A row brings its line in the conversation on screen, and that line's
   * [Show] opens this on its row, expanded (`store.showSchedule`). Cancel
   * is here only.
   */
  let open = $state(false);
  let expanded = $state<number[]>([]);

  const narrowQuery = typeof matchMedia === "function" ? matchMedia("(max-width: 640px)") : null;
  let narrow = $state(narrowQuery?.matches ?? false);
  $effect(() => {
    if (!narrowQuery) return;
    const on = () => (narrow = narrowQuery.matches);
    narrowQuery.addEventListener("change", on);
    return () => narrowQuery.removeEventListener("change", on);
  });

  const here = $derived(upcoming(store.schedules.filter((s) => s.track === store.track)));
  const others = $derived.by(() => {
    const out: { track: string; name: string; items: Watched[] }[] = [];
    for (const tr of store.tracks) {
      if (tr.id === store.track) continue;
      const items = upcoming(store.schedules.filter((s) => s.track === tr.id));
      if (items.length) out.push({ track: tr.id, name: tr.name, items });
    }
    return out;
  });
  const locale = $derived(store.lang === "ko" ? "ko-KR" : "en-US");
  const tr = (key: string, vars?: Record<string, string | number>) => t(key as Parameters<typeof t>[0], vars);

  function toggle() {
    open = !open;
    if (open) void store.loadAllSchedules();
  }

  function expand(id: number) {
    expanded = expanded.includes(id) ? expanded.filter((x) => x !== id) : [...expanded, id];
  }

  /** The schedule's line in the conversation, on screen and lit for a moment. A track just opened draws its timeline over the next frames. */
  function reveal(id: number, left = 20) {
    const el = document.getElementById(`schedule-${id}`);
    if (!el) {
      if (left > 0) setTimeout(() => reveal(id, left - 1), 100);
      return;
    }
    el.scrollIntoView({ block: "center", behavior: "smooth" });
    el.animate([{ backgroundColor: "var(--sel)" }, { backgroundColor: "transparent" }], { duration: 1400, easing: "ease-out" });
  }

  // Asked from the conversation: opened on that row, expanded, in view.
  $effect(() => {
    const focus = store.scheduleFocus;
    if (!focus) return;
    untrack(() => {
      if (!expanded.includes(focus.id)) expanded = [...expanded, focus.id];
      if (!open) {
        open = true;
        void store.loadAllSchedules();
      }
      bringRow(focus.id);
    });
  });

  /** A row scrolled to the middle of the panel and lit for a moment, once the panel has its height. */
  function bringRow(id: number, left = 20) {
    const row = document.getElementById(`schedule-row-${id}`);
    const panel = row?.closest<HTMLElement>("[role=dialog]");
    if (!row || !panel || !panel.style.maxHeight) {
      if (left > 0) setTimeout(() => bringRow(id, left - 1), 50);
      return;
    }
    panel.scrollTop = Math.max(0, row.offsetTop - (panel.clientHeight - row.offsetHeight) / 2);
    row.animate([{ backgroundColor: "var(--sel)" }, { backgroundColor: "transparent" }], { duration: 1400, easing: "ease-out" });
  }

  function go(s: Watched) {
    if (s.track === store.track) {
      reveal(s.id);
      return;
    }
    open = false;
    void store.selectTrack(s.track).then(() => reveal(s.id));
  }
</script>

{#if here.length}
  <Popover bind:open align="right" width={narrow ? 10_000 : 380} label={t("schedules.title")}>
    {#snippet trigger()}
      <button class="tog" class:on={open} type="button" title={t("schedules.title")} aria-label={t("schedules.title")} aria-haspopup="dialog" aria-expanded={open} onclick={toggle}>
        <Icon name="clock" size={14} />
        <span class="n mono" aria-hidden="true">{here.length}</span>
      </button>
    {/snippet}

    <div class="head">
      <span class="title">{t("schedules.title")}</span>
    </div>
    {#if others.length}<div class="mlab-sm group">{t("schedules.here")}</div>{/if}
    {#each here as s (s.id)}
      {@render item(s)}
    {/each}
    {#if others.length}
      <div class="mlab-sm group">{t("schedules.others")}</div>
      {#each others as g (g.track)}
        <div class="tname" title={g.name}>{g.name}</div>
        {#each g.items as s (s.id)}
          {@render item(s)}
        {/each}
      {/each}
    {/if}
  </Popover>
{/if}

{#snippet item(s: Watched)}
  {@const late = lateness(s, store.now)}
  {@const r = rule(s, tr, locale)}
  {@const text = message(s)}
  {@const full = expanded.includes(s.id)}
  {@const long = text.length > 90 || text.includes("\n")}
  <div class="row" id="schedule-row-{s.id}">
    <button class="main" type="button" onclick={() => go(s)} title={s.track === store.track ? t("schedules.show") : t("schedules.openTrack")}>
      <span class="top mono">
        <span class="id">#{s.id}</span>
        {#if s.next_at != null}
          <span class="when" class:late={late === "overdue"} title={whenFull(s.next_at, store.lang)}>{nextLabel(s.next_at, store.now, tr)}</span>
          {#if r}<span class="rule">· {r}</span>{/if}
        {:else if isCondition(s) && r}
          <span class="when cond">{r}</span>
        {/if}
        <!-- A firing that came late says so in its note, below; the tag is for one still waiting. -->
        {#if late === "overdue"}<span class="tag">{t("schedules.late")}</span>{/if}
      </span>
      <span class="msg" class:full title={full ? undefined : text}>{text}</span>
      {#if s.note}<span class="note mono" class:warn={late === "fired-late"}>{s.note}</span>{/if}
    </button>
    <div class="side">
      <button class="cancel mono" type="button" onclick={() => store.cancelSchedule(s.id)} title={t("timeline.cancelSchedule")}>{t("timeline.cancelSchedule")}</button>
      {#if long}
        <button class="more" type="button" aria-expanded={full} onclick={() => expand(s.id)} title={full ? t("schedules.less") : t("schedules.more")} aria-label={full ? t("schedules.less") : t("schedules.more")}>
          {full ? "▴" : "▾"}
        </button>
      {/if}
    </div>
  </div>
{/snippet}

<style>
  /* The panel toggle's square, widened by its count. */
  .tog {
    min-width: 28px;
    height: 28px;
    padding: 0 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--dim);
    flex-shrink: 0;
  }

  .tog:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .tog.on {
    color: var(--hi);
    border-color: var(--acc);
  }

  .n {
    font-size: 10px;
    line-height: 1;
  }

  /* The panel is a flex column that scrolls: what is in it keeps its height. */
  .head,
  .group,
  .tname,
  .row {
    flex-shrink: 0;
  }

  .head {
    display: flex;
    align-items: center;
    padding: 12px 16px 8px;
  }

  .title {
    font-size: 14px;
    color: var(--hi);
  }

  .group {
    padding: 10px 16px 6px;
  }

  .tname {
    padding: 6px 16px 4px;
    font-size: 11.5px;
    color: var(--txt);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row {
    display: flex;
    align-items: flex-start;
    border-top: 1px solid var(--lineq);
    cursor: default;
  }

  .row:hover {
    background: var(--sel);
  }

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 9px 4px 9px 16px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--dim);
  }

  .top {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 6px;
    font-size: 10.5px;
    letter-spacing: 0.04em;
    color: var(--lab);
  }

  .id {
    color: var(--lab);
  }

  .when {
    color: var(--hi);
  }

  .when.cond {
    color: var(--txt);
    overflow-wrap: anywhere;
  }

  .when.late,
  .tag {
    color: var(--warn);
  }

  .tag {
    border: 1px solid currentColor;
    padding: 0 4px;
    font-size: 9px;
    letter-spacing: 0.08em;
  }

  .msg {
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--txt);
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    overflow-wrap: anywhere;
    white-space: pre-line;
  }

  .msg.full {
    display: block;
    -webkit-line-clamp: unset;
    line-clamp: unset;
  }

  .note {
    font-size: 10px;
    color: var(--lab);
    overflow-wrap: anywhere;
  }

  .note.warn {
    color: var(--warn);
  }

  .side {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
    padding: 8px 12px 8px 4px;
    flex-shrink: 0;
  }

  /* The conversation line's own cancel button. */
  .cancel {
    padding: 2px 8px;
    background: transparent;
    border: 1px solid var(--lineq);
    color: var(--lab);
    font-size: 10px;
  }

  .cancel:hover {
    color: var(--hi);
    border-color: var(--acc);
  }

  .more {
    width: 22px;
    height: 20px;
    background: transparent;
    border: 0;
    color: var(--lab);
    font-size: 11px;
    line-height: 1;
  }

  .more:hover {
    color: var(--hi);
  }
</style>
