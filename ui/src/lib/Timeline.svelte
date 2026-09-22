<script lang="ts">
  import { store, agentLabel, type Run } from "./store.svelte";
  import Mark from "./Mark.svelte";

  let scroller = $state<HTMLDivElement>();

  // Follow the tail while a run is live.
  $effect(() => {
    void store.runs.length;
    void store.activeRun?.message.length;
    void store.activeRun?.toolCount;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });

  function secs(ms?: number) {
    if (ms == null) return "";
    return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
  }

  /** The one line a Running card is allowed to show from below the membrane. */
  function tail(run: Run) {
    const last = run.tools.at(-1);
    if (last) return `${last.toolKind} ${last.title}`;
    const text = run.message || run.thought;
    return text ? text.slice(-90).replace(/\s+/g, " ") : "대기 중…";
  }

  /** Reports are summaries; the full text lives in the lane. */
  function summary(run: Run) {
    const clean = run.message.trim().replace(/\s+/g, " ");
    return clean.length > 220 ? `${clean.slice(0, 220)}…` : clean || "(텍스트 출력 없음)";
  }
</script>

<div class="scroll" bind:this={scroller}>
  {#if store.runs.length === 0}
    <div class="empty">
      <div class="emptymark"><Mark size={56} ink="var(--lines)" /></div>
      <div class="mlab">비어 있음</div>
      <p class="serif">아래에서 레인에 첫 태스크를 보내세요.</p>
    </div>
  {/if}

  {#each store.runs as run (run.id)}
    {#if run.lane === "conductor"}
      <!-- A conductor turn: the human's message, then the conductor's reply as prose. -->
      <div class="me">
        <div class="mlab">나</div>
        <div class="bubble"><p>{run.prompt}</p></div>
      </div>
      <div class="conductor" class:live={run.status === "connecting" || run.status === "running"}>
        <div class="chead">
          <span class="mlab">지휘자</span>
          <span class="mono meta">{agentLabel(run.agent)}</span>
          {#if run.status === "connecting" || run.status === "running"}
            <span class="dot pulse"></span>
          {/if}
          <span class="grow"></span>
          <button class="btn open" onclick={() => store.inspect(run.id)}>{store.inspecting === run.id ? "닫기" : "상세"}</button>
        </div>
        {#if run.message.trim()}
          <p class="ctext">{run.message.trim()}</p>
        {:else if run.status === "failed"}
          <p class="ctext bad">{run.error}</p>
        {:else}
          <p class="ctext dim">{run.toolCount ? `레인과 작업 중 · ${run.toolCount} tools` : "생각 중…"}</p>
        {/if}
      </div>
    {:else}

    {#if run.status === "connecting" || run.status === "running"}
      <div class="card running">
        <div class="head">
          <span class="dot pulse"></span>
          <span class="mono tag ok">{run.status === "connecting" ? "CONNECTING" : "RUNNING"}</span>
          <span class="mono meta">lane {run.lane} · {run.id} · {agentLabel(run.agent)}</span>
          <span class="grow"></span>
          <span class="mono meta">{run.toolCount} tools</span>
        </div>
        <div class="mono tailline">› {tail(run)}</div>
      </div>
    {:else}
      <div class="card report" class:failed={run.status === "failed"}>
        <button class="rowhead" onclick={() => store.inspect(run.id)}>
          <span class="mono chev">{store.inspecting === run.id ? "▾" : "▸"}</span>
          <span class="col"><span class="mlab-sm">LANE</span><span class="mono val">{run.lane}</span></span>
          <span class="col"><span class="mlab-sm">RUN</span><span class="mono val">{run.id}</span></span>
          <span class="col"><span class="mlab-sm">AGENT</span><span class="mono val">{agentLabel(run.agent)}</span></span>
          <span class="col">
            <span class="mlab-sm">RESULT</span>
            <span class="mono val" class:ok={run.status === "done"} class:bad={run.status === "failed"}>
              • {run.status === "done" ? "성공" : "실패"}
            </span>
          </span>
          <span class="col"><span class="mlab-sm">TOOLS</span><span class="mono val">{run.toolCount}</span></span>
          <span class="col"><span class="mlab-sm">DURATION</span><span class="mono val">{secs(run.durationMs)}</span></span>
          <span class="grow"></span>
          <span class="mlab">REPORT</span>
        </button>

        {#if store.inspecting === run.id}
          <div class="reportbody">
            <div class="field">
              <div class="mlab-sm k">한 일</div>
              <div class="v">{summary(run)}</div>
            </div>
            {#if run.plan.length}
              <div class="field">
                <div class="mlab-sm k">계획</div>
                <div class="v">
                  {#each run.plan as entry}<div>→ {entry}</div>{/each}
                </div>
              </div>
            {/if}
            {#if run.error}
              <div class="field">
                <div class="mlab-sm k">실패</div>
                <div class="v bad">{run.error}</div>
              </div>
            {/if}
            <div class="membrane">
              <span class="mlab-sm acc">↓ MEMBRANE</span>
              <span class="note">여기까지가 Track 컨텍스트. 그 아래는 레인에 남아 있습니다.</span>
            </div>
          </div>
        {/if}
      </div>
    {/if}
    {/if}
  {/each}
</div>

<style>
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 24px 34px 28px;
  }

  .empty {
    padding: 60px 0;
    text-align: center;
  }

  .empty p {
    margin: 10px 0 0;
    font-size: 19px;
    color: var(--dim);
  }

  .emptymark {
    display: flex;
    justify-content: center;
    margin-bottom: 18px;
  }

  .me {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    margin-bottom: 22px;
  }

  .me .mlab {
    margin-bottom: 7px;
  }

  .bubble {
    max-width: 540px;
    border: 1px solid var(--accln);
    border-bottom: 2px solid var(--acc);
    background: var(--accbg);
    padding: 13px 16px;
  }

  .bubble p {
    margin: 0;
    font-family: var(--sans);
    font-size: var(--chat-fs);
    line-height: var(--chat-lh);
    color: var(--txt);
  }

  /* The conductor's turn: the same surface as the other cards, with the
     accent on the left, but its body is prose, not a report. */
  .conductor {
    max-width: 680px;
    margin-bottom: 22px;
    padding: 13px 17px 15px;
    border: 1px solid var(--line);
    border-left: 2px solid var(--lines);
    background: var(--card);
  }

  .conductor.live {
    border-left-color: var(--ok);
  }

  .chead {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 10px;
  }

  .chead .dot {
    background: var(--ok);
  }

  .open {
    height: 24px;
    padding: 0 9px;
  }

  /* Conversation text follows the interface font and the 대화창 size. */
  .ctext {
    margin: 0;
    font-family: var(--sans);
    font-size: var(--chat-fs);
    line-height: 1.7;
    color: var(--txt);
    white-space: pre-wrap;
  }

  .ctext.dim {
    color: var(--lab);
  }

  .ctext.bad {
    color: var(--acct);
  }

  .card {
    margin-bottom: 20px;
    border: 1px solid var(--line);
    background: var(--card);
  }

  .running {
    border-left: 2px solid var(--okln);
    padding: 14px 17px;
  }

  .report {
    border-left: 2px solid var(--acc);
  }

  .report.failed {
    border-left-color: var(--warn);
  }

  .head {
    display: flex;
    align-items: center;
    gap: 13px;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--ok);
  }

  .tag {
    font-size: 10px;
    letter-spacing: 0.18em;
  }

  .tag.ok {
    color: var(--ok);
  }

  .meta {
    font-size: 11px;
    color: var(--lab);
  }

  .grow {
    flex: 1;
  }

  .tailline {
    margin-top: 11px;
    font-size: 11px;
    color: var(--lab);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .rowhead {
    width: 100%;
    height: 56px;
    display: flex;
    align-items: center;
    gap: 24px;
    padding: 0 17px;
    background: transparent;
    border: 0;
    text-align: left;
  }

  .chev {
    font-size: 11px;
    color: var(--dim);
    width: 10px;
  }

  .col {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 62px;
  }

  .val {
    font-size: 12px;
    color: var(--txt);
  }

  .val.ok {
    color: var(--ok);
  }

  .val.bad {
    color: var(--acct);
  }

  .reportbody {
    border-top: 1px solid var(--line);
    padding: 18px 19px 16px;
    display: flex;
    flex-direction: column;
    gap: 15px;
  }

  .field {
    display: flex;
    gap: 20px;
  }

  .k {
    width: 66px;
    flex-shrink: 0;
    padding-top: 3px;
  }

  .v {
    font-size: calc(var(--chat-fs) - 2px);
    line-height: 1.75;
    color: var(--body);
  }

  .v.bad {
    color: var(--acct);
  }

  .membrane {
    border-top: 1px dashed var(--lines);
    padding-top: 13px;
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .acc {
    color: var(--acct);
  }

  .note {
    font-size: 12px;
    color: var(--lab);
  }
</style>
