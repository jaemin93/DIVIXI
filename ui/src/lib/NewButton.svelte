<script lang="ts">
  import type { HTMLButtonAttributes } from "svelte/elements";
  import Icon from "./Icon.svelte";

  /**
   * A list column's "+": the one button in its head that makes a new item.
   * Only the sign, since the rail and the head already say what the list
   * holds; what it makes is in its name and tooltip ("New track"), with a
   * shortcut beside it when there is one. Every list column draws this one
   * (tracks, designs, libraries, routines), so they look and act alike.
   */
  let {
    label,
    on = false,
    shortcut = "",
    ...rest
  }: {
    /** What it makes, said: its accessible name and its tooltip. */
    label: string;
    /** Its form or menu is open. */
    on?: boolean;
    /** The keys that do the same, shown in the tooltip. */
    shortcut?: string;
  } & Omit<HTMLButtonAttributes, "title" | "aria-label"> = $props();
</script>

<button
  {...rest}
  class="btn plusbtn"
  class:on
  title={shortcut ? `${label} (${shortcut})` : label}
  aria-label={label}
  aria-keyshortcuts={shortcut || undefined}
>
  <Icon name="plus" size={14} />
</button>

<style>
  /* The tracks column's old "+ TRACK" height, made square. */
  .plusbtn {
    width: 26px;
    height: 26px;
    flex-shrink: 0;
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .plusbtn.on {
    color: var(--hi);
    border-color: var(--acc);
  }
</style>
