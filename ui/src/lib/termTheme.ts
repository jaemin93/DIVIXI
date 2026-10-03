/**
 * The terminal's sixteen ANSI colours, one set per app theme.
 *
 * xterm.js ships a single palette (Tango) made for dark backgrounds; under
 * the light theme its bright yellow, bright green, bright cyan and the two
 * whites all but vanish on paper, and its bright black (the grey of `ESC[90m`)
 * is faint even on the dark one. So each theme names its own sixteen, every
 * one of them readable on that theme's `--bg`, and the panel also sets
 * `MIN_CONTRAST` so colours a program picks itself (256-colour, true
 * colour) are lifted to the same floor.
 */

export type Ansi = {
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
};

/**
 * WCAG AA for body text. xterm halves it for dim (`ESC[2m`) text, which
 * still keeps faint text legible instead of letting it fade into the page.
 */
export const MIN_CONTRAST = 4.5;

/** Tango, as xterm.js has it, with the two dark-on-dark colours lifted. */
const dark: Ansi = {
  black: "#2e3436",
  red: "#e0443c",
  green: "#4e9a06",
  yellow: "#c4a000",
  blue: "#5c8fd6",
  magenta: "#a77aa9",
  cyan: "#06989a",
  white: "#d3d7cf",
  brightBlack: "#8e8e8e",
  brightRed: "#ef2929",
  brightGreen: "#8ae234",
  brightYellow: "#fce94f",
  brightBlue: "#729fcf",
  brightMagenta: "#ad7fa8",
  brightCyan: "#34e2e2",
  brightWhite: "#eeeeec",
};

/**
 * Ink on paper: every hue deep enough for `#faf9f7`. The "whites" turn
 * into greys, as light terminal themes do, since white text is meant to
 * stand out from the background, not match it.
 */
const light: Ansi = {
  black: "#1a1917",
  red: "#b3261e",
  green: "#1f7a3d",
  yellow: "#8a6100",
  blue: "#1f57b8",
  magenta: "#8e3fa6",
  cyan: "#0b7285",
  white: "#5e5b55",
  brightBlack: "#6b675f",
  brightRed: "#c42016",
  brightGreen: "#2b7a2b",
  brightYellow: "#8f5806",
  brightBlue: "#2563c9",
  brightMagenta: "#a3389e",
  brightCyan: "#0e7480",
  brightWhite: "#45423d",
};

export function ansiFor(theme: "dk" | "lt"): Ansi {
  return theme === "lt" ? light : dark;
}

/** Relative luminance of `#rrggbb`, per WCAG 2. */
function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast ratio between two `#rrggbb` colours, 1 to 21. */
export function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
