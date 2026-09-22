<script lang="ts">
  import { store, type ThemePref } from "./store.svelte";
  import { t } from "./i18n.svelte";

  const options = $derived<{ id: ThemePref; label: string; note: string }[]>([
    { id: "system", label: t("theme.system"), note: t("theme.systemNote") },
    { id: "dark", label: t("theme.dark"), note: t("theme.darkNote") },
    { id: "light", label: t("theme.light"), note: t("theme.lightNote") },
  ]);
</script>

<div class="picker" role="radiogroup" aria-label={t("theme.label")}>
  {#each options as o (o.id)}
    <button
      class="opt"
      class:on={store.themePref === o.id}
      role="radio"
      aria-checked={store.themePref === o.id}
      onclick={() => store.setTheme(o.id)}
    >
      <span class="swatch" data-theme={o.id}>
        <span class="sw dk"></span><span class="sw lt"></span>
      </span>
      <span class="mono label">{o.label}</span>
      <span class="note">{o.note}</span>
    </button>
  {/each}
</div>

<style>
  .picker {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    border: 1px solid var(--line);
  }

  .opt {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 16px 16px 14px;
    background: transparent;
    border: 0;
    border-right: 1px solid var(--line);
    border-bottom: 2px solid transparent;
    text-align: left;
    color: var(--dim);
  }

  .opt:last-child {
    border-right: 0;
  }

  .opt:hover {
    background: var(--sel);
  }

  .opt.on {
    border-bottom-color: var(--acc);
    color: var(--hi);
    background: var(--sel);
  }

  /* Two swatches; system shows both halves. */
  .swatch {
    display: flex;
    width: 44px;
    height: 28px;
    border: 1px solid var(--lines);
    overflow: hidden;
  }

  .sw {
    flex: 1;
  }

  .sw.dk {
    background: #0b0b0b;
  }

  .sw.lt {
    background: #faf9f7;
  }

  .swatch[data-theme="dark"] .lt,
  .swatch[data-theme="light"] .dk {
    display: none;
  }

  .label {
    font-size: 10px;
    letter-spacing: 0.18em;
  }

  .note {
    font-size: 11px;
    color: var(--lab);
  }
</style>
