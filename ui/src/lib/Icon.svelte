<script module lang="ts">
  /** 16px stroke icons for the rail. Hairline weight, square joins, no fill. */
  export type IconName =
    | "tracks"
    | "lanes"
    | "draft"
    | "wrapup"
    | "agents"
    | "settings"
    | "collapse"
    | "expand";
</script>

<script lang="ts">
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.25"
  stroke-linecap="square"
  stroke-linejoin="miter"
  aria-hidden="true"
>
  {#if name === "tracks"}
    <path d="M2 4h12M2 8h12M2 12h8" />
  {:else if name === "lanes"}
    <path d="M4 2v12M8 2v12M12 2v8" />
  {:else if name === "draft"}
    <path d="M3 13l1-4 7-7 3 3-7 7-4 1z" /><path d="M9.5 4.5l2 2" />
  {:else if name === "wrapup"}
    <path d="M3 3h10v10H3z" /><path d="M3 8h10M8 8v5" />
  {:else if name === "agents"}
    <rect x="2.5" y="2.5" width="4" height="4" /><rect x="9.5" y="2.5" width="4" height="4" />
    <rect x="2.5" y="9.5" width="4" height="4" /><rect x="9.5" y="9.5" width="4" height="4" />
  {:else if name === "settings"}
    <path d="M2 4.5h12M2 11.5h12" /><rect x="5" y="2.5" width="3" height="4" /><rect x="8" y="9.5" width="3" height="4" />
  {:else if name === "collapse"}
    <path d="M9 3L4 8l5 5M12 3v10" />
  {:else if name === "expand"}
    <path d="M7 3l5 5-5 5M4 3v10" />
  {/if}
</svg>

<style>
  svg {
    display: block;
    flex-shrink: 0;
  }
</style>
