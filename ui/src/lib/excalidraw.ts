/**
 * Excalidraw, loaded on first use (it brings React and a few MB with it),
 * with its fonts served by the app rather than a CDN (`/excalidraw/fonts`,
 * see vite.config.ts), and the scene helpers the design board needs.
 */
export type ExcalidrawModule = typeof import("@excalidraw/excalidraw");

/** Excalidraw's imperative API, as its component hands it over. */
export type BoardApi = Parameters<NonNullable<import("react").ComponentProps<ExcalidrawModule["Excalidraw"]>["excalidrawAPI"]>>[0];

/** Mirrors `design::Scene`. */
export type DesignScene = {
  version: number;
  elements: readonly Record<string, unknown>[];
  files: Record<string, unknown>;
};

/** Mirrors `design::DesignDelta`: elements the agent changed. */
export type DesignDelta = { id: string; version: number; elements: Record<string, unknown>[] };

let loading: Promise<ExcalidrawModule> | null = null;

export function loadExcalidraw(): Promise<ExcalidrawModule> {
  if (!loading) {
    (window as unknown as { EXCALIDRAW_ASSET_PATH: string }).EXCALIDRAW_ASSET_PATH = `${location.origin}/excalidraw/`;
    loading = Promise.all([import("@excalidraw/excalidraw"), import("@excalidraw/excalidraw/index.css")]).then(([m]) => m);
  }
  return loading;
}

/** The largest side of a board picture sent to an agent or attached. */
const PICTURE_MAX = 1600;

/**
 * A picture of a scene as base64 PNG, on a white page, or "" when there is
 * nothing to draw.
 */
export async function scenePng(elements: readonly Record<string, unknown>[], files: Record<string, unknown>): Promise<string> {
  const live = elements.filter((e) => !e.isDeleted);
  if (!live.length) return "";
  const ex = await loadExcalidraw();
  const restored = ex.restoreElements(live as never, null, { refreshDimensions: true, repairBindings: true });
  const blob = await ex.exportToBlob({
    elements: restored,
    files: files as never,
    appState: { exportBackground: true, viewBackgroundColor: "#ffffff" },
    mimeType: "image/png",
    exportPadding: 24,
    getDimensions: (width: number, height: number) => {
      const scale = Math.min(2, PICTURE_MAX / Math.max(width, height, 1));
      return { width: width * scale, height: height * scale, scale };
    },
  });
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(binary);
}

/** The fills that mean something on a design (the agent is told the same). */
export const MEANING_FILLS = { goal: "#b2f2bb", constraint: "#ffd8a8", question: "#d0bfff", idea: "#ffec99" } as const;

/** How many live shapes carry each meaning's fill. */
export function meaningCounts(elements: readonly Record<string, unknown>[]): Record<keyof typeof MEANING_FILLS, number> {
  const out = { goal: 0, constraint: 0, question: 0, idea: 0 };
  for (const e of elements) {
    if (e.isDeleted) continue;
    const fill = String(e.backgroundColor ?? "").toLowerCase();
    for (const [k, v] of Object.entries(MEANING_FILLS)) if (fill === v) out[k as keyof typeof out] += 1;
  }
  return out;
}
