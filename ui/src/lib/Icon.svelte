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
    | "expand"
    | "overview"
    | "look"
    | "chat"
    | "info"
    | "bot"
    | "folder"
    | "library";
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
  {:else if name === "overview"}
    <rect x="2.5" y="2.5" width="11" height="11" /><path d="M2.5 6.5h11M6.5 6.5v7" />
  {:else if name === "look"}
    <circle cx="8" cy="8" r="5.5" /><path d="M8 2.5v11" /><path d="M8 2.5a5.5 5.5 0 0 1 0 11z" fill="currentColor" stroke="none" />
  {:else if name === "chat"}
    <path d="M2.5 3h11v7h-6l-3 3v-3h-2z" />
  {:else if name === "info"}
    <circle cx="8" cy="8" r="5.5" /><path d="M8 7v4M8 5v.5" />
  {:else if name === "bot"}
    <rect x="2.5" y="5.5" width="11" height="8" /><path d="M8 2.5v3M5 13.5v1M11 13.5v1" />
    <rect x="5" y="8" width="1.5" height="2" fill="currentColor" stroke="none" /><rect x="9.5" y="8" width="1.5" height="2" fill="currentColor" stroke="none" />
  {:else if name === "folder"}
    <path d="M2.5 4.5h4l1.5 1.5h5.5v7h-11z" />
  {:else if name === "library"}
    <path d="M2.5 2.5h3v11h-3zM6.5 2.5h3v11h-3zM10.5 3.5l2.8-.8 2.2 10-2.8.8z" />
  {/if}
</svg>

<style>
  svg {
    display: block;
    flex-shrink: 0;
  }
</style>
