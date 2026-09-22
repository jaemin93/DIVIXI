<script module lang="ts">
  /**
   * The membrane mark: lanes below a hairline, one thing above it.
   *
   * It is the product's claim drawn once — many things happen below, one
   * crosses — and it doubles as a status display: each lane stroke can
   * carry a state, so the same mark that sits in the title bar shows
   * detection progress on the setup panel.
   */
  export type LaneState = "plain" | "idle" | "probing" | "ready" | "attention" | "error";
</script>

<script lang="ts">
  let {
    size = 16,
    lanes = ["plain", "plain", "plain", "plain"] as LaneState[],
    /** Colour of the mark above the line. Defaults to the accent. */
    dot = "var(--acc)",
    /** Colour of the hairline and plain lanes. */
    ink = "currentColor",
    title = "",
  }: {
    size?: number;
    lanes?: LaneState[];
    dot?: string;
    ink?: string;
    title?: string;
  } = $props();

  const colors: Record<LaneState, string> = {
    plain: ink,
    idle: "var(--idle)",
    probing: "var(--idle)",
    ready: "var(--ok)",
    attention: "var(--warn)",
    error: "var(--acct)",
  };

  // Geometry on a 64-unit box. Lanes spread evenly under the line.
  const n = $derived(Math.max(1, lanes.length));
  const xs = $derived(lanes.map((_, i) => (n === 1 ? 32 : 14 + (36 * i) / (n - 1))));
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 64 64"
  fill="none"
  aria-hidden={title ? undefined : "true"}
  role={title ? "img" : undefined}
  shape-rendering="crispEdges"
>
  {#if title}<title>{title}</title>{/if}
  <!-- what crossed -->
  <rect x="29.5" y="15" width="5" height="5" fill={dot} />
  <!-- the membrane -->
  <rect x="8" y="30" width="48" height="1.5" fill={ink} />
  <!-- the lanes -->
  {#each lanes as state, i (i)}
    <rect
      x={xs[i] - 1.75}
      y="38"
      width="3.5"
      height="18"
      fill={colors[state]}
      class:pulse={state === "probing"}
    />
  {/each}
</svg>

<style>
  svg {
    display: block;
    flex-shrink: 0;
  }
</style>
