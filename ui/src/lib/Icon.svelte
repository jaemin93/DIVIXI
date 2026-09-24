<script module lang="ts">
  /** 16px stroke icons for the rail. Hairline weight, square joins, no fill. */
  export type IconName =
    | "tracks"
    | "workers"
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
    | "panel"
    | "changes"
    | "files"
    | "refresh"
    | "close"
    | "filter"
    | "terminal"
    | "book";
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
  {:else if name === "workers"}
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
  {:else if name === "panel"}
    <rect x="2.5" y="3" width="11" height="10" /><path d="M9.5 3v10" />
  {:else if name === "changes"}
    <circle cx="4.5" cy="4" r="1.5" /><circle cx="4.5" cy="12" r="1.5" /><circle cx="11.5" cy="12" r="1.5" /><path d="M4.5 5.5v5M11.5 10.5V8L7 5" />
  {:else if name === "files"}
    <path d="M2.5 3.5h3l1 1.5h7v8h-11z" /><path d="M2.5 7.5h11" />
  {:else if name === "refresh"}
    <path d="M13 8a5 5 0 1 1-1.5-3.5" /><path d="M13 2.5v3h-3" />
  {:else if name === "close"}
    <path d="M4 4l8 8M12 4l-8 8" />
  {:else if name === "terminal"}
    <rect x="1.5" y="2.5" width="13" height="11" /><path d="M4.5 6l2 2-2 2M8 10.5h3.5" />
  {:else if name === "book"}
    <path d="M8 4.5C6.5 3.3 4.5 3 2.5 3.5v9c2-.5 4-.2 5.5 1 1.5-1.2 3.5-1.5 5.5-1v-9c-2-.5-4-.2-5.5 1z" /><path d="M8 4.5v9" />
  {:else if name === "filter"}
    <path d="M2.5 4.5h11M4.5 8h7M6.5 11.5h3" />
  {/if}
</svg>

<style>
  svg {
    display: block;
    flex-shrink: 0;
  }
</style>
