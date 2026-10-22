/**
 * What the remote picker (FolderPicker.svelte, picking files) decides
 * without a screen: paths, which entries may be picked, the selection.
 * The listing itself comes from the instance (`browse_files`).
 */

/** One entry of a folder, as `browse_files` answers it. */
export type PickEntry = { name: string; dir: boolean; size: number };

/** `name` inside `base`, with the separator `base` already uses. */
export function joinPath(base: string, name: string): string {
  if (base.endsWith("/") || base.endsWith("\\")) return base + name;
  return `${base}${base.includes("\\") ? "\\" : "/"}${name}`;
}

/** The folder a picked path is in, or "" for a bare name. */
export function folderOf(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (i < 0) return "";
  // A root keeps its separator: `C:\`, `/`.
  const head = path.slice(0, i);
  return head === "" || /^[A-Za-z]:$/.test(head) ? path.slice(0, i + 1) : head;
}

/** The extension, lowercased, without the dot ("" for none, or a dotfile). */
export function extOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}

/** A file's size the way the files panel shows it. */
export function sizeLabel(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}

/**
 * Whether an entry may be picked: a file, of a kind in `formats` (the
 * library's extensions). With no list known yet, any file: the instance
 * still refuses what it cannot read, and says why.
 */
export function canPick(e: PickEntry, formats: readonly string[]): boolean {
  if (e.dir) return false;
  return formats.length === 0 || formats.includes(extOf(e.name));
}

/** The selection with `path` added, or taken out when it was in. */
export function toggle(selected: readonly string[], path: string): string[] {
  return selected.includes(path) ? selected.filter((p) => p !== path) : [...selected, path];
}
