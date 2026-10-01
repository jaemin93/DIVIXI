<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, onServerInstall, type ServerInstall } from "./ipc.svelte";
  import { t, type Key } from "./i18n.svelte";

  /**
   * divixi-server on a remote machine: whether it is there, whether it is
   * current, and putting it there when the person says so.
   *
   * The work is `src-tauri/src/remote/install.rs`; this shows what it found
   * and how far it has got. Nothing is written to anyone's machine
   * unasked -- 207 MB is not a thing to start on its own -- so every case
   * ends in a sentence and a button, and an update can be told "don't ask
   * again" per instance.
   *
   * Dropped into the instance's card in Settings and into the page shown
   * when an instance could not be reached; it draws nothing at all when
   * there is nothing to say.
   */
  let {
    id,
    /** What the instance's own build said, for the "another build" sentence. */
    build = null,
    /**
     * Look at the remote as soon as this appears. True where something is
     * already known to be wrong (a connection that failed, an instance
     * reporting another build); false in a list, where it would mean an SSH
     * round trip per row.
     */
    auto = false,
    /** Shown after an install, when reconnecting is the next thing to do. */
    onInstalled = undefined,
  }: {
    id: string;
    build?: string | null;
    auto?: boolean;
    onInstalled?: () => void;
  } = $props();

  type Source = "app" | "newest";
  type Why = "older_release" | "other_build" | "too_old";
  type Check =
    | { kind: "unsupported"; os: string; arch: string }
    | { kind: "missing"; target: string; source: Source; size: number }
    | { kind: "outdated"; why: Why; installed: string | null; target: string; source: Source; size: number }
    | { kind: "current"; installed: string | null; running: boolean }
    | { kind: "unmanaged"; target: string; source: Source; size: number; running: boolean }
    | { kind: "managed"; exec_start: string; want: string }
    | { kind: "no_target"; detail: string }
    | { kind: "no_shell" };
  type Found = Check & { muted: boolean };

  let found = $state<Found | null>(null);
  let looking = $state(false);
  let error = $state("");
  let progress = $state<ServerInstall | null>(null);
  /** Starting a server that is there and down. */
  let starting = $state(false);
  /** The path setting was pointed at the managed binary. */
  let repointed = $state(false);

  /**
   * Megabytes as the release notes and docs/divixi-server.md count them, so
   * the same binary is not "207 MB" on the page and "198 MB" on screen.
   */
  const mb = (bytes: number) => Math.round(bytes / 1_000_000);

  const AT: Record<ServerInstall["phase"], Key> = {
    checking: "srv.at.checking",
    downloading: "srv.at.downloading",
    verifying: "srv.at.verifying",
    installing: "srv.at.installing",
    starting: "srv.at.starting",
    done: "srv.at.installing",
    failed: "srv.at.installing",
    cancelled: "srv.at.downloading",
  };

  const PHASE: Record<ServerInstall["phase"], Key> = {
    checking: "srv.phase.checking",
    downloading: "srv.phase.downloading",
    verifying: "srv.phase.verifying",
    installing: "srv.phase.installing",
    starting: "srv.phase.starting",
    done: "srv.phase.done",
    failed: "srv.phase.downloading",
    cancelled: "srv.phase.cancelled",
  };

  /** An install is underway: no other button applies while it is. */
  const busy = $derived(progress !== null && !["done", "failed", "cancelled"].includes(progress.phase));
  /** The release it would install, when there is one. */
  const target = $derived(found && "target" in found ? found.target : "");
  const size = $derived(found && "size" in found ? found.size : 0);
  /** This app is a development build, so no release can make the builds agree. */
  const devBuild = $derived(found !== null && "source" in found && found.source === "newest");

  async function look() {
    looking = true;
    error = "";
    try {
      found = await invoke<Found>("remote_server_check", { id });
    } catch (err) {
      error = String(err);
    } finally {
      looking = false;
    }
  }

  async function start() {
    error = "";
    repointed = false;
    progress = { host: id, phase: "checking", at: null, received: 0, total: null, route: null, release: null, error: null };
    try {
      await invoke<string>("remote_server_install", { id });
      repointed = true;
      // Not reconnected here: the person reads "installed and running"
      // first, and presses for it. A reload would take the sentence away
      // before anyone saw it.
      await look();
    } catch (err) {
      // The events carry the phase and the remote's own words; this is the
      // one line the command itself returned, kept for the case where no
      // event arrived at all.
      if (!progress?.error) error = String(err);
    }
  }

  /** It is installed and simply not running. */
  async function startServer() {
    starting = true;
    error = "";
    try {
      await invoke("remote_server_start", { id });
      onInstalled?.();
      await look();
    } catch (err) {
      error = `${t("srv.startFailed")}
${String(err)}`;
    } finally {
      starting = false;
    }
  }

  function cancel() {
    void invoke("remote_server_cancel", { id });
  }

  /** Out of a finished state, back to what the remote looks like now. */
  async function again() {
    progress = null;
    await look();
  }

  async function mute(muted: boolean) {
    try {
      await invoke("remote_server_mute", { id, muted });
      if (found) found = { ...found, muted };
    } catch (err) {
      error = String(err);
    }
  }

  onMount(() => {
    if (auto) void look();
    const stop = onServerInstall((p) => {
      if (p.host === id) progress = p;
    });
    return () => void stop.then((f) => f());
  });
</script>

<div class="si">
  {#if progress}
    <!-- Underway, or just finished: the phases, in order, with the reason
         when one of them failed. -->
    <div class="row">
      <span class="what" class:bad={progress.phase === "failed"} class:ok={progress.phase === "done"}>
        {progress.phase === "done" ? t("srv.phase.done", { release: progress.release ?? target }) : t(PHASE[progress.phase])}
      </span>
      {#if progress.phase === "downloading"}
        <span class="mono num">
          {#if progress.total}
            {t("srv.progress", { done: mb(progress.received), total: mb(progress.total), pct: Math.min(100, Math.floor((progress.received / progress.total) * 100)) })}
          {:else}
            {t("srv.progressUnknown", { done: mb(progress.received) })}
          {/if}
        </span>
      {/if}
      {#if busy}
        <button class="btn sm" onclick={cancel}>{t("srv.cancel")}</button>
      {/if}
    </div>
    {#if progress.phase === "downloading" && progress.route}
      <div class="bar" role="progressbar" aria-valuemin={0} aria-valuemax={progress.total ?? 0} aria-valuenow={progress.received}>
        <span class="fill" class:idle={!progress.total} style:width={progress.total ? `${Math.min(100, (progress.received / progress.total) * 100)}%` : "100%"}></span>
      </div>
      <p class="note">{progress.route === "remote" ? t("srv.route.remote") : t("srv.route.ssh")}</p>
    {/if}
    {#if progress.phase === "failed"}
      <p class="note">{t("srv.failedAt", { phase: t(AT[progress.at ?? "installing"]) })}</p>
      {#if progress.error}<pre class="mono why">{progress.error}</pre>{/if}
      <div class="acts"><button class="btn sm" onclick={start}>{t("srv.retry")}</button></div>
    {/if}
    {#if progress.phase === "cancelled"}
      <!-- Cancelled on purpose: the ask comes back, rather than the install. -->
      <div class="acts"><button class="btn sm" onclick={again}>{t("srv.check")}</button></div>
    {/if}
    {#if progress.phase === "done"}
      {#if repointed}<p class="note">{t("srv.binPath")}</p>{/if}
      {#if onInstalled}<div class="acts"><button class="btn sm btn-acc" onclick={() => onInstalled?.()}>{t("srv.reconnect")}</button></div>{/if}
    {/if}
  {:else if looking}
    <p class="note">{t("srv.checking")}</p>
  {:else if error}
    <p class="note">{t("srv.checkFailed")}</p>
    <pre class="mono why">{error}</pre>
    <div class="acts"><button class="btn sm" onclick={look}>{t("srv.retry")}</button></div>
  {:else if found === null}
    <div class="acts"><button class="btn sm" onclick={look}>{t("srv.check")}</button></div>
  {:else if found.kind === "missing"}
    <p class="say">{t("srv.missing", { release: found.target })}</p>
    <p class="note">{t("srv.willWrite", { mb: mb(size) })}</p>
    {#if devBuild}<p class="note warn">{t("srv.devBuild")}</p>{/if}
    <div class="acts"><button class="btn sm btn-acc" onclick={start}>{t("srv.install")}</button></div>
  {:else if found.kind === "outdated"}
    {#if found.muted}
      <div class="row">
        <span class="note">{t("srv.muted")}</span>
        <button class="btn sm" onclick={start}>{t("srv.update")}</button>
        <button class="btn sm" onclick={() => mute(false)}>{t("srv.unmute")}</button>
      </div>
    {:else}
      <p class="say warn">
        {#if found.why === "older_release"}
          {found.installed ? t("srv.older", { installed: found.installed, release: found.target }) : t("srv.olderUnknown", { release: found.target })}
        {:else if found.why === "too_old"}
          {t("srv.tooOld", { release: found.target })}
        {:else}
          {t("srv.otherBuild", { build: build ?? "?", release: found.target })}
        {/if}
      </p>
      <p class="note">{t("srv.willWrite", { mb: mb(size) })}</p>
      {#if devBuild}<p class="note warn">{t("srv.devBuild")}</p>{/if}
      <div class="acts">
        <button class="btn sm btn-acc" onclick={start}>{t("srv.update")}</button>
        <button class="btn sm" onclick={() => mute(true)}>{t("srv.dontAsk")}</button>
      </div>
    {/if}
  {:else if found.kind === "current"}
    <p class="note">{found.installed ? t("srv.current", { release: found.installed }) : t("srv.currentBuild")}</p>
    {#if !found.running}
      <!-- Installed and down. Starting it is the whole fix; putting 207 MB
           over a binary that is already the right one is not. -->
      <p class="note warn">{t("srv.stopped")}</p>
      <div class="acts"><button class="btn sm btn-acc" disabled={starting} onclick={startServer}>{starting ? t("srv.starting") : t("srv.start")}</button></div>
    {/if}
  {:else if found.kind === "unmanaged"}
    <p class="note">{t("srv.unmanaged")}</p>
    {#if !found.running}<p class="note warn">{t("srv.stopped")}</p>{/if}
    <p class="note">{t("srv.willWrite", { mb: mb(size) })}</p>
    {#if devBuild}<p class="note warn">{t("srv.devBuild")}</p>{/if}
    <div class="acts">
      {#if !found.running}<button class="btn sm btn-acc" disabled={starting} onclick={startServer}>{starting ? t("srv.starting") : t("srv.start")}</button>{/if}
      <button class="btn sm" onclick={start}>{t("srv.installAnyway", { release: found.target })}</button>
    </div>
  {:else if found.kind === "unsupported"}
    <p class="note">{t("srv.unsupported", { os: found.os, arch: found.arch })}</p>
    <p class="note mono">{t("srv.manual")}</p>
  {:else if found.kind === "managed"}
    <p class="note warn">{t("srv.managed", { path: found.exec_start })}</p>
    <div class="acts"><button class="btn sm" onclick={look}>{t("srv.check")}</button></div>
  {:else if found.kind === "no_target"}
    <p class="note">{t("srv.noTarget", { why: found.detail })}</p>
    <div class="acts"><button class="btn sm" onclick={look}>{t("srv.retry")}</button></div>
  {:else if found.kind === "no_shell"}
    <p class="note">{t("srv.noShell")}</p>
    <p class="note mono">{t("srv.manual")}</p>
  {/if}
</div>

<style>
  .si {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .say {
    margin: 0;
    font-size: 12.5px;
    line-height: 1.6;
    color: var(--txt);
  }

  .note {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.6;
    color: var(--dim);
  }

  .warn {
    color: var(--warn);
  }

  .what {
    font-size: 12.5px;
    color: var(--txt);
  }

  .what.ok {
    color: var(--ok);
  }

  .what.bad {
    color: var(--deltx);
  }

  .num {
    font-size: 11.5px;
    color: var(--dim);
  }

  /* The bar is the only moving thing here; it never appears without a phase
     beside it saying what is moving. */
  .bar {
    height: 3px;
    background: var(--inp);
    border: 1px solid var(--lineq);
  }

  .fill {
    display: block;
    height: 100%;
    background: var(--acc);
    transition: width 120ms linear;
  }

  /* No total to divide by: full width, and the words carry the state. */
  .fill.idle {
    opacity: 0.4;
  }

  .why {
    max-width: 100%;
    margin: 0;
    font-size: 11px;
    line-height: 1.55;
    color: var(--deltx);
    white-space: pre-wrap;
    /* The remote's own words, selectable: they go into an issue as they are. */
    user-select: text;
  }

  .acts {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .btn.sm {
    height: 26px;
    padding: 0 10px;
  }
</style>
