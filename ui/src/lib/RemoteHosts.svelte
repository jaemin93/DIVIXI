<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import { t } from "./i18n.svelte";

  /**
   * Divixis on other machines (a Linux server running divixi-server), used
   * from this app: the app starts divixi-server there if need be, opens an
   * SSH tunnel, and shows that Divixi in a window of its own (as Kiro Crew's
   * remote hosts). A host is added or changed in a dialog laid out as Kiro
   * Crew's: host, binary path, port, PATH, then Save and Cancel.
   */
  type Host = { id: string; name: string; ssh: string; port: number; bin: string; path: string; local_port: number | null };

  const PORT = 7488;
  const BIN = "~/.local/bin/divixi-server";

  let hosts = $state<Host[]>([]);
  let editing = $state<Host | null>(null);
  let busy = $state("");
  let error = $state("");
  let formError = $state("");
  let first = $state<HTMLInputElement>();

  const blank = (): Host => ({ id: "", name: "", ssh: "", port: PORT, bin: BIN, path: "", local_port: null });

  onMount(async () => {
    try {
      hosts = await invoke<Host[]>("remote_hosts");
    } catch (err) {
      store.lastError = String(err);
    }
  });

  $effect(() => {
    first?.focus();
  });

  function open(h: Host) {
    formError = "";
    editing = { ...h };
  }

  function close() {
    editing = null;
    formError = "";
  }

  async function save(e: Event) {
    e.preventDefault();
    if (!editing) return;
    formError = "";
    try {
      const { local_port: _lp, ...host } = editing;
      // The list names a host by its SSH target, as Kiro Crew does.
      const port = Number(host.port) || PORT;
      hosts = await invoke<Host[]>("remote_host_save", { host: { ...host, name: host.ssh, port, bin: host.bin.trim() || BIN } });
      editing = null;
    } catch (err) {
      formError = String(err);
    }
  }

  async function act(cmd: "remote_host_connect" | "remote_host_disconnect" | "remote_host_delete", id: string) {
    busy = id;
    error = "";
    try {
      hosts = await invoke<Host[]>(cmd, { id });
    } catch (err) {
      error = String(err);
    } finally {
      busy = "";
    }
  }

  function onKey(e: KeyboardEvent) {
    if (editing && e.key === "Escape") close();
  }

  function onBackdrop(e: MouseEvent) {
    if (e.target === e.currentTarget) close();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="rh">
  <div class="line">
    <div class="what">
      <div class="title">{t("remote.hosts.title")}</div>
      <div class="dim">{t("remote.hosts.note")}</div>
    </div>
    <button class="btn" onclick={() => open(blank())}>{t("remote.hosts.add")}</button>
  </div>

  {#each hosts as h (h.id)}
    <div class="host">
      <span class="dot" class:on={h.local_port !== null}></span>
      <div class="what">
        <div class="title mono">{h.ssh}</div>
        <div class="mono dim">:{h.port} · {h.bin}{#if h.local_port !== null} · {t("remote.hosts.tunnel", { port: h.local_port })}{/if}</div>
      </div>
      <button class="btn sm btn-acc" disabled={busy === h.id} onclick={() => act("remote_host_connect", h.id)}>
        {busy === h.id ? t("remote.hosts.connecting") : h.local_port !== null ? t("remote.hosts.open") : t("remote.hosts.connect")}
      </button>
      {#if h.local_port !== null}
        <button class="btn sm" disabled={busy === h.id} onclick={() => act("remote_host_disconnect", h.id)}>{t("remote.hosts.disconnect")}</button>
      {/if}
      <button class="btn sm" onclick={() => open(h)}>{t("remote.hosts.edit")}</button>
      <button class="btn sm" onclick={() => act("remote_host_delete", h.id)}>{t("remote.hosts.delete")}</button>
    </div>
  {/each}

  {#if error}<p class="err mono">{error}</p>{/if}
</div>

{#if editing}
  <div class="backdrop" onclick={onBackdrop} role="presentation">
    <div class="dlg" role="dialog" aria-modal="true" aria-label={t("remote.hosts.title")}>
      <form class="sheet" onsubmit={save}>
        <label>
          <span class="lab">{t("remote.hosts.ssh")}</span>
          <input class="mono" bind:this={first} bind:value={editing.ssh} placeholder="user@myhost.example.com" spellcheck="false" autocomplete="off" />
          <span class="hint">{t("remote.hosts.sshNote")}</span>
        </label>
        <label>
          <span class="lab">{t("remote.hosts.bin")}</span>
          <input class="mono" bind:value={editing.bin} placeholder={BIN} spellcheck="false" autocomplete="off" />
        </label>
        <label>
          <span class="lab">{t("remote.hosts.port")} <span class="def">{t("remote.hosts.portDefault", { port: PORT })}</span></span>
          <input class="mono" type="number" min="1024" max="65535" bind:value={editing.port} placeholder={String(PORT)} />
        </label>
        <label>
          <span class="lab">{t("remote.hosts.path")} <span class="def">{t("remote.hosts.pathDefault")}</span></span>
          <input class="mono" bind:value={editing.path} placeholder="~/.local/bin:/usr/local/bin" spellcheck="false" autocomplete="off" />
        </label>
        {#if formError}<p class="err mono">{formError}</p>{/if}
        <div class="acts">
          <button class="btn btn-acc" type="submit" disabled={!editing.ssh.trim()}>{t("remote.hosts.save")}</button>
          <button class="btn" type="button" onclick={close}>{t("remote.cancel")}</button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  .rh {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 640px;
    margin-top: 26px;
    padding-top: 18px;
    border-top: 1px solid var(--line);
  }

  .line,
  .host {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .host {
    padding: 8px 0;
    border-bottom: 1px solid var(--lineq);
  }

  .what {
    flex: 1;
    min-width: 0;
  }

  .title {
    font-size: 13px;
    color: var(--hi);
  }

  .dim {
    font-size: 11.5px;
    color: var(--dim);
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--idle);
    flex-shrink: 0;
  }

  .dot.on {
    background: var(--ok);
  }

  .btn.sm {
    height: 26px;
    padding: 0 10px;
  }

  /* The host dialog, in the middle of the window (as TagDialog). */
  .backdrop {
    position: fixed;
    inset: 32px 0 0 0;
    z-index: 40;
    background: color-mix(in srgb, var(--bg) 60%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }

  .dlg {
    width: min(620px, 100%);
    max-height: 100%;
    display: flex;
  }

  .sheet {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 22px;
    margin: 0;
    padding: 30px 32px 28px;
    background: var(--card);
    border: 1px solid var(--lines);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .lab {
    font-size: 14px;
    color: var(--txt);
  }

  .def {
    color: var(--lab);
  }

  .hint {
    font-size: 12px;
    color: var(--dim);
    line-height: 1.5;
  }

  input {
    height: 44px;
    padding: 0 14px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-size: 14px;
    outline: none;
  }

  input:focus {
    border-color: var(--acc);
  }

  input::placeholder {
    color: var(--lab);
  }

  .acts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    margin-top: 2px;
  }

  .acts .btn {
    height: 40px;
    justify-content: center;
    font-size: 13px;
    font-weight: 600;
  }

  .err {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--deltx);
    white-space: pre-wrap;
  }
</style>
