/**
 * A QR code, as a grid of modules, and that grid as one SVG path.
 *
 * The encoding is `qrcode-generator` (Kazuhiko Arase, MIT, no dependencies) —
 * the reference implementation, and the most read one there is. This was a
 * hand-written encoder for one release and it did not produce a readable
 * symbol: version and size came out right and 22% of the modules did not, so
 * a phone camera fell back to reading the picture as text. A QR code is a
 * published standard with no product judgement in it and nothing here to own.
 *
 * What is ours is the drawing: one path for the whole symbol rather than a
 * rect per module, so the theme picks the colours and a version 12 code is
 * one element instead of two thousand.
 */

import qrcode from "qrcode-generator";

/** Error correction M: about 15% recovery, the usual choice for a screen. */
const EC_LEVEL = "M";

/** `0` lets the library pick the smallest version the payload fits. */
const AUTO_VERSION = 0;

export type Qr = { size: number; modules: boolean[][] };

/**
 * Encode `text` as a QR code at level M, in the smallest version that fits.
 *
 * ASCII only, and that is checked rather than assumed: the library's byte
 * conversion is `charCodeAt(i) & 0xff`, which truncates anything above U+00FF
 * without saying so. Divixi's only payload is a pairing URL — a DNS name, a
 * fixed path and base64url — so this never bites in practice, and the throw is
 * here so it cannot start biting quietly.
 */
export function encodeQr(text: string): Qr {
  const bad = [...text].find((c) => c.codePointAt(0)! > 0x7f);
  if (bad !== undefined) {
    throw new Error(`a QR payload must be ASCII; found ${JSON.stringify(bad)}`);
  }
  const qr = qrcode(AUTO_VERSION, EC_LEVEL);
  qr.addData(text, "Byte");
  qr.make();
  const size = qr.getModuleCount();
  const modules = Array.from({ length: size }, (_, r) => Array.from({ length: size }, (_, c) => qr.isDark(r, c)));
  return { size, modules };
}

/**
 * The dark modules as one SVG path, in a viewBox of `size + 2 * quiet`.
 *
 * The quiet zone is part of the symbol, not decoration: a reader needs clear
 * space around the finders. The standard asks for four modules; two is enough
 * for a code read off a lit screen and keeps it compact on a card.
 */
export function qrPath(qr: Qr, quiet = 2): string {
  const parts: string[] = [];
  for (let r = 0; r < qr.size; r++) {
    for (let c = 0; c < qr.size; c++) {
      if (qr.modules[r][c]) parts.push(`M${c + quiet} ${r + quiet}h1v1h-1z`);
    }
  }
  return parts.join("");
}
