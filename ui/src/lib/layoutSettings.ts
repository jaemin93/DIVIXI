/**
 * Settings that are a window's layout, not the person's preferences or the
 * work: what is open, how wide, how far zoomed, where it was.
 *
 * Settings live on the PC and every window reads the same ones, so a phone
 * that kept these would rearrange the PC's window (and a phone-sized choice
 * makes no sense on a desktop, nor the other way round). Only this PC's own
 * Divixi keeps them; anywhere else they last as long as the page.
 */
export const LAYOUT_SETTINGS = [
  "panel",
  "panel_width",
  "terminal",
  "terminal_height",
  "rail",
  "rail_width",
  "tracklist",
  "tracklist_width",
  "tracks_filter",
  "designlist",
  "designlist_width",
  "designchat",
  "kblist",
  "kblist_width",
  "kblibrary",
  "artifact_chat_width",
  "settings_nav_width",
  "routinelist_width",
  "track",
  "track_headers",
  "ws_open",
] as const;

/**
 * Type and its size: chosen for the screen in front of you, so each device
 * has its own, as the layout is. Theme and language stay shared: they are
 * the person's, whatever the screen.
 */
export const TYPE_SETTINGS = ["ui_font", "chat_font", "zoom"] as const;

export function isLayoutSetting(key: string): boolean {
  return (LAYOUT_SETTINGS as readonly string[]).includes(key);
}

export function isTypeSetting(key: string): boolean {
  return (TYPE_SETTINGS as readonly string[]).includes(key);
}

/**
 * Whether a window keeps this setting: writes it, and reads it back at start.
 * Only this PC's own Divixi keeps the layout and the type; anywhere else they
 * start from the defaults (a phone's panel closed, the default type) and last
 * as long as the page.
 */
export function keepsSetting(key: string, local: boolean): boolean {
  return local || !(isLayoutSetting(key) || isTypeSetting(key));
}
