<script lang="ts">
  import { onMount } from "svelte";
  import { invoke, instanceId, switchInstance, onInstanceStatus } from "./ipc.svelte";
  import { store } from "./store.svelte";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n.svelte";

  /**
   * Settings › Remote instances: Divixis on other machines, shown in this
   * window from the header's switcher. Reached over SSH (divixi-server
   * started there if need be, a tunnel, a pairing token minted over SSH)
   * or at an address (a Divixi serving on its network, which lets in this
   * PC's GitHub account if it is its owner's). Laid out as Kiro Crew's
   * page: a card per instance, then adding one; the form is a dialog.
   */
  type Host = {
    id: string;
    name: string;
    kind: "ssh" | "direct";
    url: string;
    ssh: string;
    port: number;
    bin: string;
    path: string;
    local_port: number | null;
    connected: boolean;
    online: boolean;
  };

  const PORT = 7488;
  const BIN = "~/.local/bin/divixi-server";

  let hosts = $state<Host[]>([]);
  let editing = $state<Host | null>(null);
  let busy = $state("");
  let error = $state("");
  let formError = $state("");
  let confirmDelete = $state("");
  let first = $state<HTMLInputElement>();

  const blank = (): Host => ({ id: "", name: "", kind: "ssh", url: "", ssh: "", port: PORT, bin: BIN, path: "", local_port: null, connected: false, online: false });

  onMount(() => {
    invoke<Host[]>("remote_hosts")
      .then((h) => (hosts = h))
      .catch((err) => (store.lastError = String(err)));
    const stop = onInstanceStatus(({ id, online }) => {
      hosts = hosts.map((h) => (h.id === id ? { ...h, online, connected: h.connected || online } : h));
    });
    return () => void stop.then((f) => f());
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
      const { local_port: _lp, connected: _c, online: _o, ...host } = editing;
      const port = Number(host.port) || PORT;
      // An empty name takes the one the field suggests.
      const name = host.name.trim() || t("remote.namePlaceholder", { n: hosts.length + 1 });
      hosts = await invoke<Host[]>("remote_host_save", { host: { ...host, name, port, bin: host.bin.trim() || BIN } });
      editing = null;
    } catch (err) {
      formError = String(err);
    }
  }

  async function act(cmd: "remote_host_connect" | "remote_host_disconnect" | "remote_host_delete", id: string) {
    busy = id;
    error = "";
    confirmDelete = "";
    try {
      hosts = await invoke<Host[]>(cmd, { id });
      // The instance on screen is gone: back to this PC.
      if (cmd !== "remote_host_connect" && id === instanceId) switchInstance(null);
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
  <section class="card">
    <div class="chead">
      <Icon name="remote" />
      <span class="ctitle">{t("remote.list")}</span>
      <span class="mono count">{hosts.length}</span>
    </div>

    {#each hosts as h (h.id)}
      {@const on = h.connected}
      <div class="host">
        <span class="badge"><Icon name="remote" /></span>
        <div class="what">
          <div class="name">{h.name}</div>
          <div class="meta mono">
            {#if h.kind === "direct"}<span class="tag">{t("remote.kindDirectTag")}</span>{h.url}
            {:else}<span class="tag">SSH</span>{h.ssh} · {t("remote.remotePort", { port: h.port })}{/if}
          </div>
          <div class="state" class:on={h.online} class:trying={on && !h.online}>
            <span class="dot"></span>{#if !on}{t("remote.notConnected")}{:else if !h.online}{t("instances.link.trying")}{:else if h.local_port !== null}{t("remote.connected", { port: h.local_port })}{:else}{t("instances.link.online")}{/if}
          </div>
        </div>
        <div class="acts">
          {#if instanceId === h.id}
            <span class="viewing">{t("remote.viewing")}</span>
          {:else}
            <button class="btn sm btn-acc" onclick={() => switchInstance(h.id)}>{t("remote.show")}</button>
          {/if}
          {#if !on && instanceId !== h.id}
            <button class="btn sm" disabled={busy === h.id} onclick={() => act("remote_host_connect", h.id)}>
              {busy === h.id ? t("remote.connecting") : t("remote.connect")}
            </button>
          {/if}
          {#if on}
            <button class="btn sm" disabled={busy === h.id} onclick={() => act("remote_host_disconnect", h.id)}>{t("remote.disconnect")}</button>
          {/if}
          <button class="btn sm" onclick={() => open(h)}>{t("remote.edit")}</button>
          {#if confirmDelete === h.id}
            <button class="btn sm danger" onclick={() => act("remote_host_delete", h.id)}>{t("remote.deleteConfirm")}</button>
          {:else}
            <button class="btn sm" onclick={() => (confirmDelete = h.id)}>{t("remote.delete")}</button>
          {/if}
        </div>
      </div>
    {:else}
      <div class="empty">{t("remote.empty")}</div>
    {/each}

    {#if error}<p class="err mono">{error}</p>{/if}
  </section>

  <button class="card add" onclick={() => open(blank())}>
    <span class="plus">+</span>
    <span class="ctitle">{t("remote.add")}</span>
  </button>

  <p class="note">{t("remote.how")}</p>
</div>

{#if editing}
  <div class="backdrop" onclick={onBackdrop} role="presentation">
    <div class="dlg" role="dialog" aria-modal="true" aria-labelledby="remote-dlg-title">
      <form class="sheet" onsubmit={save}>
        <div class="dtitle" id="remote-dlg-title">{editing.id ? t("remote.editTitle") : t("remote.add")}</div>
        <label>
          <span class="lab">{t("remote.name")}</span>
          <input bind:this={first} bind:value={editing.name} placeholder={t("remote.namePlaceholder", { n: hosts.length + 1 })} autocomplete="off" />
        </label>
        <label>
          <span class="lab">{t("remote.kind")}</span>
          <select bind:value={editing.kind}>
            <option value="ssh">{t("remote.kindSsh")}</option>
            <option value="direct">{t("remote.kindDirect")}</option>
          </select>
          <span class="hint">{editing.kind === "direct" ? t("remote.kindDirectHint") : t("remote.kindSshHint")}</span>
        </label>
        {#if editing.kind === "direct"}
          <label>
            <span class="lab">{t("remote.url")}</span>
            <input class="mono" bind:value={editing.url} placeholder="http://my-pc:7488" spellcheck="false" autocomplete="off" />
            <span class="hint">{t("remote.urlHint")}</span>
          </label>
        {:else}
          <label>
            <span class="lab">{t("remote.ssh")}</span>
            <input class="mono" bind:value={editing.ssh} placeholder="host-1-alias" spellcheck="false" autocomplete="off" />
            <span class="hint">{t("remote.sshHint")}</span>
          </label>
          <label>
            <span class="lab">{t("remote.port")} <span class="def">{t("remote.default", { value: PORT })}</span></span>
            <input class="mono" type="number" min="1024" max="65535" bind:value={editing.port} placeholder={String(PORT)} />
            <span class="hint">{t("remote.portHint")}</span>
          </label>
          <label>
            <span class="lab">{t("remote.bin")}</span>
            <input class="mono" bind:value={editing.bin} placeholder={BIN} spellcheck="false" autocomplete="off" />
          </label>
          <label>
            <span class="lab">{t("remote.path")} <span class="def">{t("remote.pathDefault")}</span></span>
            <input class="mono" bind:value={editing.path} placeholder="~/.local/bin:/usr/local/bin" spellcheck="false" autocomplete="off" />
            <span class="hint">{t("remote.pathHint")}</span>
          </label>
        {/if}
        {#if formError}<p class="err mono">{formError}</p>{/if}
        <div class="dacts">
          <button class="btn btn-acc" type="submit" disabled={!(editing.kind === "direct" ? editing.url : editing.ssh).trim()}>{t("remote.save")}</button>
          <button class="btn" type="button" onclick={close}>{t("remote.cancel")}</button>
        </div>
      </form>
    </div>
  </div>
{/if}

<style>
  /* Full width of the pane, so its right edge meets the Close button's. */
  .rh {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .card {
    border: 1px solid var(--line);
    background: var(--card);
  }

  .chead {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 18px;
    border-bottom: 1px solid var(--line);
    color: var(--dim);
  }

  .ctitle {
    font-size: 14px;
    color: var(--hi);
  }

  .count {
    font-size: 11px;
    color: var(--lab);
  }

  .host {
    display: flex;
    align-items: flex-start;
    gap: 14px;
    padding: 16px 18px;
    border-top: 1px solid var(--lineq);
  }

  .chead + .host {
    border-top: 0;
  }

  .badge {
    width: 36px;
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--line);
    background: var(--inp);
    color: var(--dim);
  }

  .what {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .name {
    font-size: 14px;
    color: var(--hi);
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
    color: var(--dim);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .tag {
    padding: 1px 6px;
    border: 1px solid var(--line);
    font-size: 10px;
    letter-spacing: 0.08em;
    color: var(--txt);
  }

  .state {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: var(--lab);
  }

  .state.on {
    color: var(--txt);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--idle);
  }

  .state.on .dot {
    background: var(--ok);
  }

  .state.trying {
    color: var(--warn);
  }

  .state.trying .dot {
    background: var(--warn);
  }

  .acts {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }

  .btn.sm {
    height: 28px;
    padding: 0 12px;
  }

  .viewing {
    align-self: center;
    padding: 0 6px;
    font-size: 12px;
    color: var(--acct);
  }

  .danger {
    color: var(--deltx);
    border-color: var(--deltx);
  }

  .empty {
    padding: 18px;
    font-size: 12.5px;
    color: var(--lab);
  }

  .add {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 16px 18px;
    text-align: left;
    color: var(--dim);
  }

  .add:hover {
    background: var(--sel);
  }

  .plus {
    width: 16px;
    font-size: 17px;
    line-height: 1;
    text-align: center;
  }

  .note {
    margin: 4px 0 0;
    font-size: 12.5px;
    line-height: 1.65;
    color: var(--dim);
  }

  .err {
    margin: 0;
    padding: 0 18px 14px;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--deltx);
    white-space: pre-wrap;
  }

  /* The instance dialog, in the middle of the window (as TagDialog). */
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
    gap: 20px;
    margin: 0;
    padding: 26px 32px 28px;
    background: var(--card);
    border: 1px solid var(--lines);
  }

  .dtitle {
    font-size: 16px;
    color: var(--hi);
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .lab {
    font-size: 13.5px;
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

  select {
    height: 42px;
    padding: 0 10px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-size: 14px;
  }

  input {
    height: 42px;
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

  .dacts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    margin-top: 2px;
  }

  .dacts .btn {
    height: 40px;
    justify-content: center;
    font-size: 13px;
    font-weight: 600;
  }

  .sheet .err {
    padding: 0;
  }
</style>
