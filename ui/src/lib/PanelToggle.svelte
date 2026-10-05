<script lang="ts">
  import Icon from "./Icon.svelte";

  /**
   * The right-hand panel's toggle: the same square, the same place (the top
   * right of a page's head) on every page that has one — the working folder
   * beside a track, the agent beside a design or the knowledge page.
   */
  let { on, title, onclick }: { on: boolean; title: string; onclick: () => void } = $props();
</script>

<button class="tog" class:on type="button" {title} aria-label={title} aria-pressed={on} {onclick}>
  <Icon name="panel" size={14} />
</button>

<style>
  .tog {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
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
</style>
