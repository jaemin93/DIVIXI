<script lang="ts">
  import { store } from "./store.svelte";
  import { t, type LangPref } from "./i18n.svelte";

  const options = $derived<{ id: LangPref; label: string; note: string }[]>([
    { id: "system", label: t("lang.system"), note: t("lang.systemNote") },
    { id: "ko", label: t("lang.ko"), note: "ko" },
    { id: "en", label: t("lang.en"), note: "en" },
  ]);
</script>

<div class="picker" role="radiogroup" aria-label={t("lang.label")}>
  {#each options as o (o.id)}
    <button
      class="opt"
      class:on={store.langPref === o.id}
      role="radio"
      aria-checked={store.langPref === o.id}
      onclick={() => store.setLang(o.id)}
    >
      <span class="label">{o.label}</span>
      <span class="mono note">{o.note}</span>
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
    gap: 6px;
    padding: 14px 16px 12px;
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

  .label {
    font-size: 14px;
  }

  .note {
    font-size: 10px;
    letter-spacing: 0.14em;
    color: var(--lab);
  }
</style>
