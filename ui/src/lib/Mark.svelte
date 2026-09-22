<script lang="ts">
  /**
   * The staff mark: four staves and one downbeat.
   *
   * The staves are the lanes the work runs along; the vertical bar is the
   * conductor's beat that cuts across all of them. It is a brand mark, not
   * a gauge: it does not count agents or lanes. The only thing it shows is
   * whether the conductor is live (`live` pulses the bar).
   */
  let {
    size = 16,
    /** Colour of the staves. */
    ink = "currentColor",
    /** Colour of the downbeat. Defaults to the accent. */
    beat = "var(--acc)",
    /** Pulse the downbeat while a run is in flight. */
    live = false,
    title = "",
  }: {
    size?: number;
    ink?: string;
    beat?: string;
    live?: boolean;
    title?: string;
  } = $props();

  // Geometry on a 64-unit box.
  const staves = [22, 30, 38, 46];
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
  {#each staves as y (y)}
    <rect x="8" y={y - 0.75} width="48" height="1.5" fill={ink} />
  {/each}
  <!-- the downbeat -->
  <rect x="21" y="13" width="3.5" height="42" fill={beat} class:pulse={live} />
</svg>

<style>
  svg {
    display: block;
    flex-shrink: 0;
  }
</style>
