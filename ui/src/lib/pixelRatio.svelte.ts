/**
 * The display's device pixel ratio, kept current.
 *
 * It changes when the window moves to a screen with another scale, or when
 * the page is zoomed; `matchMedia` on the current resolution says when, and
 * is asked again for the new one. One listener for the whole app, read by
 * every mark (Mark.svelte lays its staves out in device pixels).
 */
class PixelRatio {
  value = $state(typeof window === "undefined" ? 1 : window.devicePixelRatio || 1);

  constructor() {
    if (typeof window === "undefined" || typeof matchMedia !== "function") return;
    const watch = () => {
      matchMedia(`(resolution: ${this.value}dppx)`).addEventListener(
        "change",
        () => {
          this.value = window.devicePixelRatio || 1;
          watch();
        },
        { once: true },
      );
    };
    watch();
  }
}

export const pixelRatio = new PixelRatio();
