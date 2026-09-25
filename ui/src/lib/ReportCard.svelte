<script lang="ts">
  import { store, type WorkerReport } from "./store.svelte";
  import { t } from "./i18n.svelte";
  import WorkerChanges from "./WorkerChanges.svelte";

  /**
   * A worker's report as it crossed the membrane: where the work stands, a
   * summary, and counts of changes, checks and questions; the lists open on
   * demand. Questions always show: they are what someone has to answer.
   */
  let {
    run,
    worker = "",
    compact = false,
    track = "",
    latest = false,
  }: {
    run: string;
    worker?: string;
    compact?: boolean;
    /** With `latest`, the worker's unmerged changes show under its newest report. */
    track?: string;
    latest?: boolean;
  } = $props();

  const report = $derived<WorkerReport | undefined>(store.reports[run]);
  let open = $state(false);

  $effect(() => {
    // Asked again whenever a report reaches the conductor: this one may be it.
    void store.reportsArrived;
    if (!store.reports[run]) void store.loadReport(run);
  });

  const passed = $derived(report?.checks.filter((c) => c.result === "pass").length ?? 0);
  const failed = $derived(report?.checks.filter((c) => c.result === "fail").length ?? 0);
  const claimed = $derived(report?.changes.map((c) => c.path) ?? []);
  /** Edits the app saw that the report does not list. */
  const unlisted = $derived((report?.edits_seen ?? []).filter((seen) => !claimed.some((c) => seen.endsWith(c) || c.endsWith(seen))));
  const hasMore = $derived(!!report && (report.changes.length + report.checks.length + report.risks.length + report.next.length + unlisted.length > 0));
</script>

{#if report}
  <section class="report s-{report.status}" class:compact>
    <div class="head">
      <span class="mlab kind">{t("report.label")}{worker ? ` · ${worker}` : ""}</span>
      <span class="mono state">{t(`report.status.${report.status}`)}</span>
      {#if !report.structured}<span class="mono state loose" title={report.problem ?? ""}>{t("report.unstructured")}</span>{/if}
      <span class="grow"></span>
      <button class="mono runid" type="button" onclick={() => worker && store.openWorkerView(worker)} disabled={!worker} title={t("report.openWorker")}>{run}</button>
    </div>

    <p class="summary">{report.summary}</p>

    {#if report.questions.length}
      <div class="questions">
        <div class="mlab-sm">{t("report.questions")}</div>
        <ul>
          {#each report.questions as q, i (i)}<li>{q}</li>{/each}
        </ul>
      </div>
    {/if}

    <div class="facts mono">
      <span>{t("report.changes", { n: report.changes.length })}</span>
      {#if report.structured}
        {#if report.checks.length === 0}
          <span class="warnf">{t("report.noChecks")}</span>
        {:else}
          <span class:bad={failed > 0} class:good={failed === 0}>{t("report.checks", { passed, total: report.checks.length })}</span>
        {/if}
      {/if}
      {#if report.risks.length}<span class="warnf">{t("report.risks", { n: report.risks.length })}</span>{/if}
      {#if unlisted.length}<span class="warnf" title={unlisted.join("\n")}>{t("report.unlisted", { n: unlisted.length })}</span>{/if}
      {#if hasMore}
        <button type="button" class="more" onclick={() => (open = !open)} aria-expanded={open}>{open ? t("report.less") : t("report.more")}</button>
      {/if}
    </div>

    {#if open}
      <div class="details">
        {#if report.changes.length}
          <div class="mlab-sm">{t("report.changesTitle")}</div>
          <ul>
            {#each report.changes as c, i (i)}
              {@const rel = store.relativeToTrack(c.path) ?? c.path}
              <li>
                <button type="button" class="mono path" onclick={() => store.openFile(rel)}>{c.path}</button>
                {#if c.what}<span class="what">{c.what}</span>{/if}
              </li>
            {/each}
          </ul>
        {/if}
        {#if report.checks.length}
          <div class="mlab-sm">{t("report.checksTitle")}</div>
          <ul class="checks">
            {#each report.checks as c, i (i)}
              <li>
                <span class="mono mark r-{c.result}">{c.result === "pass" ? "✓" : c.result === "fail" ? "✗" : "–"}</span>
                <span class="mono">{c.what}</span>
                {#if c.detail}<span class="what">{c.detail}</span>{/if}
              </li>
            {/each}
          </ul>
        {/if}
        {#if report.risks.length}
          <div class="mlab-sm">{t("report.risksTitle")}</div>
          <ul>
            {#each report.risks as r, i (i)}<li>{r}</li>{/each}
          </ul>
        {/if}
        {#if unlisted.length}
          <div class="mlab-sm">{t("report.unlistedTitle")}</div>
          <ul>
            {#each unlisted as p (p)}<li><button type="button" class="mono path" onclick={() => store.openFile(p)}>{p}</button></li>{/each}
          </ul>
        {/if}
        {#if report.next.length}
          <div class="mlab-sm">{t("report.next")}</div>
          <ul>
            {#each report.next as n, i (i)}<li>{n}</li>{/each}
          </ul>
        {/if}
      </div>
    {/if}

    {#if latest && track && worker}
      <WorkerChanges {track} {worker} />
    {/if}
  </section>
{/if}

<style>
  .report {
    max-width: 680px;
    margin: -6px 0 22px;
    padding: 11px 16px 12px;
    border: 1px solid var(--line);
    border-left: 2px solid var(--lines);
    background: var(--card);
  }

  .report.compact {
    margin: 10px 0 0;
  }

  .s-done {
    border-left-color: var(--ok);
  }

  .s-partial,
  .s-blocked {
    border-left-color: var(--warn);
  }

  .s-failed {
    border-left-color: var(--deltx);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .state {
    font-size: 9px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--lab);
  }

  .s-done .state:not(.loose) {
    color: var(--ok);
  }

  .s-partial .state:not(.loose),
  .s-blocked .state:not(.loose) {
    color: var(--warn);
  }

  .s-failed .state:not(.loose) {
    color: var(--deltx);
  }

  .grow {
    flex: 1;
  }

  .runid {
    font-size: 10px;
    color: var(--lab);
    background: none;
    border: none;
    padding: 0;
    cursor: pointer;
  }

  .runid:hover:not(:disabled) {
    color: var(--acc);
  }

  .summary {
    margin: 7px 0 0;
    font-family: var(--sans);
    font-size: var(--chat-fs);
    line-height: 1.55;
    color: var(--txt);
    white-space: pre-wrap;
  }

  .questions {
    margin-top: 9px;
    padding: 7px 10px;
    border: 1px solid var(--warnln);
    background: var(--warnbg);
  }

  .questions .mlab-sm {
    color: var(--warn);
  }

  ul {
    margin: 4px 0 0;
    padding-left: 18px;
    font-family: var(--sans);
    font-size: calc(var(--chat-fs) - 1px);
    line-height: 1.55;
    color: var(--txt);
  }

  .checks {
    list-style: none;
    padding-left: 0;
  }

  .checks li {
    display: flex;
    gap: 8px;
    align-items: baseline;
  }

  .mark {
    width: 12px;
    flex-shrink: 0;
    text-align: center;
  }

  .r-pass {
    color: var(--ok);
  }

  .r-fail {
    color: var(--deltx);
  }

  .r-not_run {
    color: var(--lab);
  }

  .facts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 14px;
    margin-top: 9px;
    font-size: 10.5px;
    color: var(--dim);
  }

  .facts .good {
    color: var(--ok);
  }

  .facts .bad {
    color: var(--deltx);
  }

  .facts .warnf {
    color: var(--warn);
  }

  .more {
    margin-left: auto;
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: var(--lab);
    cursor: pointer;
  }

  .more:hover {
    color: var(--acc);
  }

  .details {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--line);
  }

  .details .mlab-sm {
    margin-top: 8px;
  }

  .details .mlab-sm:first-child {
    margin-top: 0;
  }

  .path {
    background: none;
    border: none;
    padding: 0;
    font-size: 11.5px;
    color: var(--hi);
    cursor: pointer;
    text-align: left;
  }

  .path:hover {
    color: var(--acc);
  }

  .what {
    margin-left: 8px;
    color: var(--dim);
  }
</style>
