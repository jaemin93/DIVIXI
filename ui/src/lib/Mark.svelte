<script lang="ts">
  /**
   * The Divixi mark: four staves and an X across them.
   *
   * The staves are the lanes the work runs along; the X is the conductor's
   * beat cutting across all of them, and the x in the name: crossing
   * providers, one becoming many. It is a brand mark, not a gauge: it does
   * not count agents or lanes. The only thing it shows is whether the
   * conductor is live (`live` pulses the X).
   *
   * Drawn as a tile with its own colours, the same in both themes and the
   * same as the app icon (`scripts/make-icon.mjs` uses this geometry), so
   * what sits in the rail is what sits in the taskbar.
   */
  let {
    size = 16,
    /** Pulse the X while a run is in flight. */
    live = false,
    title = "",
  }: {
    size?: number;
    live?: boolean;
    title?: string;
  } = $props();

  // Geometry on a 64-unit box; the icon script draws the same numbers.
  const staves = [22, 30, 38, 46];
  // Fixed colours. The tile sits halfway between the dark theme's page
  // (#0b0b0b) and the light theme's (#faf9f7), so it reads as a tile on both.
  const TILE = "#808080";
  const EDGE = "#8f8f8f";
  const PALE = "#f7f6f4";
  const ACCENT = "#e03127";
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 64 64"
  fill="none"
  aria-hidden={title ? undefined : "true"}
  role={title ? "img" : undefined}
>
  {#if title}<title>{title}</title>{/if}
  <rect x="0" y="0" width="64" height="64" fill={TILE} />
  <rect x="0.5" y="0.5" width="63" height="63" fill="none" stroke={EDGE} stroke-width="1" vector-effect="non-scaling-stroke" />
  {#each staves as y (y)}
    <rect x="8" y={y - 0.75} width="48" height="1.5" fill={PALE} shape-rendering="crispEdges" />
  {/each}
  <!-- the X: the beat across the staves -->
  <path d="M20 12 L44 56 M44 12 L20 56" stroke={ACCENT} stroke-width="4" stroke-linecap="square" class:pulse={live} />
</svg>

<style>
  svg {
    display: block;
    flex-shrink: 0;
  }
</style>
