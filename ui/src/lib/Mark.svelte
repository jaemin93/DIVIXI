<script lang="ts">
  /**
   * The Divixi mark: four staves and an X across them.
   *
   * The staves are the lines the work runs along; the X is the conductor's
   * beat cutting across all of them, and the x in the name: crossing
   * providers, one becoming many. It is a brand mark, not a gauge: it does
   * not count agents or workers. The only thing it shows is whether the
   * conductor is live (`live` turns the X).
   *
   * Drawn with its own colours on no background, the same in both themes
   * and the same weight as the app icon (`scripts/make-icon.mjs`: staves 2
   * units thick, never under a pixel), so what sits in the rail is what sits
   * in the taskbar. The staves are laid out in whole device pixels
   * (markGeometry.ts), so all four show at every size and display scale; the
   * X keeps the 64-unit geometry.
   */
  import { staves } from "./markGeometry";
  import { pixelRatio } from "./pixelRatio.svelte";

  let {
    size = 16,
    /** Turn the X while a run is in flight. */
    live = false,
    title = "",
  }: {
    size?: number;
    live?: boolean;
    title?: string;
  } = $props();

  // No tile: the staves are a deep beige that reads on the dark theme's page
  // and on the light theme's alike, and the X is the accent. Fixed colours,
  // so the rail and the app icon are one drawing.
  const STAVE = "#a8843d";
  const ACCENT = "#e03127";

  // The viewBox is the box in device pixels: one unit, one pixel.
  const st = $derived(staves(size, pixelRatio.value));
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 {st.box} {st.box}"
  fill="none"
  aria-hidden={title ? undefined : "true"}
  role={title ? "img" : undefined}
>
  {#if title}<title>{title}</title>{/if}
  {#each st.rows as y (y)}
    <rect x={st.x0} {y} width={st.x1 - st.x0} height={st.thick} fill={STAVE} shape-rendering="crispEdges" />
  {/each}
  <!-- the X: the beat across the staves, two bars that turn each on their own -->
  <g transform="scale({st.box / 64})">
    <path d="M20 12 L44 56" stroke={ACCENT} stroke-width="4" stroke-linecap="square" class="bar a" class:spin={live} />
    <path d="M44 12 L20 56" stroke={ACCENT} stroke-width="4" stroke-linecap="square" class="bar b" class:spin={live} />
  </g>
</svg>

<style>
  svg {
    display: block;
    flex-shrink: 0;
  }

  /* Each bar turns about its own centre, which for both is the X's (32, 34);
     the staves stay. */
  .bar {
    transform-box: fill-box;
    transform-origin: center;
  }

  /* Working: one bar breathes -- slow off the mark, fast through the turn,
     slow into the X again -- while the other keeps an even pace. They meet
     in the X every 1.4 s. */
  .a.spin {
    animation: turn 1.4s cubic-bezier(0.65, 0, 0.35, 1) infinite;
  }

  .b.spin {
    animation: turn 1.4s linear infinite;
  }

  @keyframes turn {
    to {
      transform: rotate(360deg);
    }
  }

  /* Reduced motion: nothing turns. The X stays as drawn and one bar fades
     in and out, so working still reads as working. */
  @media (prefers-reduced-motion: reduce) {
    .b.spin {
      animation: none;
    }

    .a.spin {
      animation: breathe 2.4s ease-in-out infinite alternate;
    }
  }

  @keyframes breathe {
    to {
      opacity: 0.4;
    }
  }
</style>
