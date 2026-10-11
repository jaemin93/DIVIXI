/**
 * A colour a person picked, as it may go into a `style`: `#rrggbb` only,
 * anything else (a value from an older build, a remote instance) none. The
 * core checks colours as they are saved; this is the page's own check.
 */
export function colorOrNone(color: string | null | undefined): string {
  return color && /^#[0-9a-f]{6}$/i.test(color) ? color : "transparent";
}
