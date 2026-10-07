/**
 * A design as a file, the window's side: what a browser may send, and how a
 * file it is handed is saved. The file itself is written and read in the core
 * (src-tauri/src/design_file.rs); this only gets it there and back.
 */

/** The file's extension, as the core names it. */
export const DESIGN_EXT = ".divixi-design";

/** The largest file the PC reads (the core's MAX_PACKAGE). */
export const IMPORT_MAX_MB = 140;

/**
 * The largest a browser may send: the PC's server takes 72 MB a request,
 * and the file goes as JSON text inside one.
 */
export const WEB_IMPORT_MAX_MB = 70;

/** Too large to import from here, in MB for the message; or null when it may go. */
export function tooLarge(bytes: number, overWeb: boolean): number | null {
  const max = overWeb ? WEB_IMPORT_MAX_MB : IMPORT_MAX_MB;
  return bytes > max * 1024 * 1024 ? max : null;
}

/** Hand a browser a file to keep, as its download. */
export function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}
