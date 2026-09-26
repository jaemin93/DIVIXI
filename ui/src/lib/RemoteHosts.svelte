<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import { t } from "./i18n.svelte";

  /**
   * Divixis on other machines (a Linux server running divixi-server), used
   * from this app: the app opens an SSH tunnel, fetches a pairing link over
   * SSH, and shows that Divixi in a window of its own (as Kiro Crew's
   * remote hosts).
   */
  type Host = { id: string; name: string; ssh: string; port: number; bin: string; local_port: number | null };

  let hosts = $state<Host[]>([]);
  let editing = $state<Host | null>(null);
  let busy = $state("");
  let error = $state("");

  const blank = (): Host => ({ id: "", name: "", ssh: "", port: 7488, bin: "~/.local/bin/divixi-server", local_port: null });

  onMount(async () => {
    try {
      hosts = await invoke<Host[]>("remote_hosts");
    } catch (err) {
      store.lastError = String(err);
    }
  });

  async function save() {
    if (!editing) return;
    error = "";
    try {
      const { local_port: _lp, ...host } = editing;
      hosts = await invoke<Host[]>("remote_host_save", { host: { ...host, port: Number(host.port) } });
      editing = null;
    } catch (err) {
      error = String(err);
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
</script>

<div class="rh">
  <div class="line">
    <div class="what">
      <div class="title">{t("remote.hosts.title")}</div>
      <div class="dim">{t("remote.hosts.note")}</div>
    </div>
    {#if !editing}<button class="btn" onclick={() => (editing = blank())}>{t("remote.hosts.add")}</button>{/if}
  </div>

  {#each hosts as h (h.id)}
    <div class="host">
      <span class="dot" class:on={h.local_port !== null}></span>
      <div class="what">
        <div class="title">{h.name}</div>
        <div class="mono dim">{h.ssh} · :{h.port}{#if h.local_port !== null} · {t("remote.hosts.tunnel", { port: h.local_port })}{/if}</div>
      </div>
      <button class="btn sm btn-acc" disabled={busy === h.id} onclick={() => act("remote_host_connect", h.id)}>
        {busy === h.id ? t("remote.hosts.connecting") : h.local_port !== null ? t("remote.hosts.open") : t("remote.hosts.connect")}
      </button>
      {#if h.local_port !== null}
        <button class="btn sm" disabled={busy === h.id} onclick={() => act("remote_host_disconnect", h.id)}>{t("remote.hosts.disconnect")}</button>
      {/if}
      <button class="btn sm" onclick={() => (editing = { ...h })}>{t("remote.hosts.edit")}</button>
      <button class="btn sm" onclick={() => act("remote_host_delete", h.id)}>{t("remote.hosts.delete")}</button>
    </div>
  {/each}

  {#if editing}
    <div class="form">
      <label>
        <span class="mlab-sm">{t("remote.hosts.name")}</span>
        <input bind:value={editing.name} placeholder="Linux server" />
      </label>
      <label>
        <span class="mlab-sm">{t("remote.hosts.ssh")}</span>
        <input class="mono" bind:value={editing.ssh} placeholder="user@host" spellcheck="false" />
        <span class="dim">{t("remote.hosts.sshNote")}</span>
      </label>
      <div class="two">
        <label>
          <span class="mlab-sm">{t("remote.hosts.port")}</span>
          <input class="mono" type="number" min="1024" max="65535" bind:value={editing.port} />
        </label>
        <label>
          <span class="mlab-sm">{t("remote.hosts.bin")}</span>
          <input class="mono" bind:value={editing.bin} spellcheck="false" />
        </label>
      </div>
      <div class="line">
        <button class="btn btn-acc" onclick={save}>{t("remote.hosts.save")}</button>
        <button class="btn" onclick={() => ((editing = null), (error = ""))}>{t("remote.cancel")}</button>
      </div>
    </div>
  {/if}

  {#if error}<p class="err mono">{error}</p>{/if}
</div>

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

  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    background: var(--card);
    border: 1px solid var(--line);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  input {
    height: 30px;
    padding: 0 9px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-size: 12.5px;
  }

  .two {
    display: grid;
    grid-template-columns: 120px 1fr;
    gap: 10px;
  }

  .err {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--deltx);
    white-space: pre-wrap;
  }
</style>
