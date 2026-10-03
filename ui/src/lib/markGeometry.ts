/**
 * Where the mark's four staves fall, in device pixels.
 *
 * The staves are thin: on the 64-unit box they are 2 units thick, which is
 * 0.5 px at 16 px. Drawn as fractions of a pixel with crisp edges, a stave
 * lands on a pixel or misses it depending on the size, the display scale and
 * where the mark sits, so the rail showed two staves on one screen and none
 * on another. Laid out here in whole device pixels instead -- at least one
 * pixel thick, at least one empty pixel between two -- every stave covers
 * exactly the rows it should wherever the mark lands, and all four are there
 * at every size.
 *
 * Same weight as the app icon (`scripts/make-icon.mjs`): 2 units, never
 * under one pixel. Where the box is too small for the 8-unit spacing to
 * leave a gap, the staves spread just enough to keep one.
 */
export interface Staves {
  /** The box, in device pixels (the mark's CSS size times the pixel ratio). */
  box: number;
  /** Top row of each stave, in device pixels. */
  rows: number[];
  /** Stave thickness, in whole device pixels. */
  thick: number;
  /** Left and right ends, in whole device pixels. */
  x0: number;
  x1: number;
}

/** The staves of a mark `size` CSS px wide on a display at `dpr`. */
export function staves(size: number, dpr: number): Staves {
  const box = size * dpr;
  const s = box / 64;
  const thick = Math.max(1, Math.round(2 * s));
  const step = Math.max(thick + 1, Math.round(8 * s));
  const span = 3 * step + thick;
  // Centred on y = 34, the X's centre, as the 22/30/38/46 staves are.
  const top = Math.max(0, Math.min(Math.floor(box) - span, Math.round(34 * s - span / 2)));
  return {
    box,
    rows: [0, 1, 2, 3].map((i) => top + i * step),
    thick,
    x0: Math.round(8 * s),
    x1: Math.max(Math.round(8 * s) + 1, Math.round(56 * s)),
  };
}
