/**
 * The switch for the check divixi makes on its own at startup.
 *
 * The app checks once each time it starts, while the switch in
 * **Settings → About** is on. Whether a start checks is decided in
 * src-tauri/src/update.rs (`at_startup`, with its tests), because "once each
 * time the app starts" is once per process: a reloaded webview or Local shown
 * again after a remote instance is the same run, and only the process can
 * tell. There is no day's allowance and no stamp. What is left here is the
 * switch as the settings card reads and writes it.
 *
 * Where it lives: `updates.auto`, in the same settings the rest of the app
 * uses (`get_setting` / `set_setting`, which is `meta` in the store). The Rust
 * side reads the same key the same way.
 */

/** The settings key holding the switch. Absent means on. */
export const AUTO_KEY = "updates.auto";

/**
 * The switch as it is stored. Anything that is not the word `off` is on:
 * a machine that has never touched the setting checks, and a value nobody
 * recognises is not a reason to stop.
 */
export function autoFrom(raw: string | null | undefined): boolean {
  return raw !== "off";
}

/** The switch, to store. */
export function autoTo(on: boolean): string {
  return on ? "on" : "off";
}
