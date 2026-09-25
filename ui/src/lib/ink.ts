/**
 * Ink and board pictures for designs.
 *
 * Strokes are drawn with perfect-freehand (as Huabu does): points with
 * pressure become an outline polygon, filled. The agent never gets vectors;
 * it gets a picture of the board, drawn here onto a canvas and sent as a
 * PNG with each message that has ink on the board.
 */
import { getStroke } from "perfect-freehand";
import type { DesignDoc, DesignNode, Stroke } from "./store.svelte";

/** The outline of one stroke as an SVG path, in the stroke's own units. */
export function strokePath(stroke: Stroke): string {
  const outline = getStroke(stroke.points, {
    size: stroke.size,
    thinning: 0.55,
    smoothing: 0.5,
    streamline: 0.45,
    simulatePressure: stroke.points.every((p) => p[2] === 0.5),
  });
  if (outline.length < 2) return "";
  const [first, ...rest] = outline;
  let d = `M${first[0].toFixed(1)},${first[1].toFixed(1)}`;
  for (let i = 0; i < rest.length; i++) {
    const [x0, y0] = rest[i];
    const [x1, y1] = rest[(i + 1) % rest.length];
    d += ` Q${x0.toFixed(1)},${y0.toFixed(1)} ${((x0 + x1) / 2).toFixed(1)},${((y0 + y1) / 2).toFixed(1)}`;
  }
  return `${d} Z`;
}

export type Box = { x: number; y: number; w: number; h: number };

/** The box around strokes given in board units, padded by their width. */
export function strokesBox(strokes: Stroke[]): Box {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const s of strokes) {
    const pad = s.size;
    for (const [x, y] of s.points) {
      minX = Math.min(minX, x - pad);
      minY = Math.min(minY, y - pad);
      maxX = Math.max(maxX, x + pad);
      maxY = Math.max(maxY, y + pad);
    }
  }
  if (!Number.isFinite(minX)) return { x: 0, y: 0, w: 1, h: 1 };
  return { x: minX, y: minY, w: Math.max(1, maxX - minX), h: Math.max(1, maxY - minY) };
}

/** A sketch's strokes in board units. */
export function worldStrokes(node: DesignNode): Stroke[] {
  return node.strokes.map((s) => ({ ...s, points: s.points.map(([x, y, p]) => [x + node.x, y + node.y, p] as [number, number, number]) }));
}

/** Board-unit strokes made relative to a box: what a sketch stores. */
export function localStrokes(strokes: Stroke[], box: Box): Stroke[] {
  return strokes.map((s) => ({ ...s, points: s.points.map(([x, y, p]) => [x - box.x, y - box.y, p] as [number, number, number]) }));
}

export function overlaps(a: Box, b: Box, gap = 0): boolean {
  return a.x - gap < b.x + b.w && b.x - gap < a.x + a.w && a.y - gap < b.y + b.h && b.y - gap < a.y + a.h;
}

export const TAG_COLORS: Record<string, string> = {
  goal: "#46c46a",
  constraint: "#e03127",
  question: "#e0a030",
  idea: "#7aa2f7",
};

/** Where an arrow between two boxes leaves the first and meets the second. */
export function edgeEnds(a: Box, b: Box): [number, number, number, number] {
  const ca = { x: a.x + a.w / 2, y: a.y + a.h / 2 };
  const cb = { x: b.x + b.w / 2, y: b.y + b.h / 2 };
  const clip = (box: Box, from: { x: number; y: number }, to: { x: number; y: number }) => {
    const dx = to.x - from.x;
    const dy = to.y - from.y;
    const sx = dx === 0 ? Infinity : box.w / 2 / Math.abs(dx);
    const sy = dy === 0 ? Infinity : box.h / 2 / Math.abs(dy);
    const s = Math.min(sx, sy, 1);
    return { x: from.x + dx * s, y: from.y + dy * s };
  };
  const p = clip(a, ca, cb);
  const q = clip(b, cb, ca);
  return [p.x, p.y, q.x, q.y];
}

/**
 * The whole board as a PNG (base64, no prefix), for the agent to see the
 * ink. Paper background, notes as boxes with their text, strokes filled,
 * arrows as lines. Null when there is no ink: the outline says it all.
 */
export function boardPng(doc: DesignDoc, maxSide = 1600): string | null {
  if (!doc.nodes.some((n) => n.kind === "sketch")) return null;
  const pad = 40;
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const n of doc.nodes) {
    minX = Math.min(minX, n.x);
    minY = Math.min(minY, n.y);
    maxX = Math.max(maxX, n.x + n.w);
    maxY = Math.max(maxY, n.y + n.h);
  }
  const w = maxX - minX + pad * 2;
  const h = maxY - minY + pad * 2;
  const k = Math.min(1.5, maxSide / Math.max(w, h));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(w * k));
  canvas.height = Math.max(1, Math.round(h * k));
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.scale(k, k);
  ctx.translate(pad - minX, pad - minY);

  const byId = new Map(doc.nodes.map((n) => [n.id, n]));
  ctx.strokeStyle = "#8a8a8a";
  ctx.lineWidth = 2;
  for (const e of doc.edges) {
    const a = byId.get(e.from);
    const b = byId.get(e.to);
    if (!a || !b) continue;
    const [x1, y1, x2, y2] = edgeEnds(a, b);
    ctx.beginPath();
    ctx.moveTo(x1, y1);
    ctx.lineTo(x2, y2);
    ctx.stroke();
    const ang = Math.atan2(y2 - y1, x2 - x1);
    ctx.beginPath();
    ctx.moveTo(x2, y2);
    ctx.lineTo(x2 - 12 * Math.cos(ang - 0.4), y2 - 12 * Math.sin(ang - 0.4));
    ctx.lineTo(x2 - 12 * Math.cos(ang + 0.4), y2 - 12 * Math.sin(ang + 0.4));
    ctx.closePath();
    ctx.fillStyle = "#8a8a8a";
    ctx.fill();
  }

  for (const n of doc.nodes) {
    if (n.kind === "sketch") {
      ctx.save();
      ctx.translate(n.x, n.y);
      for (const s of n.strokes) {
        ctx.fillStyle = s.color || "#111111";
        ctx.fill(new Path2D(strokePath(s)));
      }
      ctx.restore();
      continue;
    }
    ctx.fillStyle = "#fafaf7";
    ctx.fillRect(n.x, n.y, n.w, n.h);
    ctx.strokeStyle = TAG_COLORS[n.tag] ?? "#bbbbbb";
    ctx.lineWidth = 2;
    ctx.strokeRect(n.x, n.y, n.w, n.h);
    ctx.fillStyle = "#111111";
    ctx.font = "15px sans-serif";
    let y = n.y + 24;
    if (n.tag) {
      ctx.fillStyle = TAG_COLORS[n.tag] ?? "#666666";
      ctx.font = "bold 11px monospace";
      ctx.fillText(n.tag.toUpperCase(), n.x + 10, y - 4);
      y += 14;
      ctx.fillStyle = "#111111";
      ctx.font = "15px sans-serif";
    }
    for (const line of wrap(ctx, n.text, n.w - 20)) {
      if (y > n.y + n.h - 6) break;
      ctx.fillText(line, n.x + 10, y);
      y += 20;
    }
  }
  return canvas.toDataURL("image/png").replace(/^data:image\/png;base64,/, "");
}

function wrap(ctx: CanvasRenderingContext2D, text: string, width: number): string[] {
  const out: string[] = [];
  for (const para of text.split("\n")) {
    let line = "";
    for (const ch of para) {
      const next = line + ch;
      if (ctx.measureText(next).width > width && line) {
        out.push(line);
        line = ch;
      } else {
        line = next;
      }
    }
    out.push(line);
  }
  return out;
}

/** The board as a brief for a new track's first message. */
export function briefOf(title: string, doc: DesignDoc, labels: Record<string, string>): string {
  const notes = doc.nodes.filter((n) => n.kind === "note" && n.text.trim());
  const section = (tag: string) => notes.filter((n) => n.tag === tag).map((n) => `- ${n.text.trim().replace(/\n+/g, " ")}`);
  const parts = [`# ${title}`];
  for (const tag of ["goal", "constraint", "question", "idea"]) {
    const lines = section(tag);
    if (lines.length) parts.push(`## ${labels[tag] ?? tag}\n${lines.join("\n")}`);
  }
  const rest = notes.filter((n) => !n.tag).map((n) => `- ${n.text.trim().replace(/\n+/g, " ")}`);
  if (rest.length) parts.push(`## ${labels.note ?? "notes"}\n${rest.join("\n")}`);
  const one = (s: string | undefined) => (s ?? "").trim().replace(/\n+/g, " ");
  const inside = (n: DesignNode, f: DesignNode) => n.x >= f.x && n.y >= f.y && n.x + n.w <= f.x + f.w && n.y + n.h <= f.y + f.h;
  const questions = doc.nodes
    .filter((n) => n.kind === "question" && n.text.trim())
    .map((n) => `- ${one(n.text)}\n  - ${labels.answer ?? "answer"}: ${one(n.answer) || (labels.unanswered ?? "(open)")}`);
  if (questions.length) parts.push(`## ${labels.questions ?? "questions"}\n${questions.join("\n")}`);
  const links = doc.nodes
    .filter((n) => n.kind === "link" && n.url)
    .map((n) => `- [${one(n.name) || n.url}](${n.url})${one(n.text) ? ` — ${one(n.text)}` : ""}`);
  if (links.length) parts.push(`## ${labels.links ?? "links"}\n${links.join("\n")}`);
  const files = doc.nodes
    .filter((n) => n.kind === "file")
    .map((n) => `- ${one(n.name) || n.src}${one(n.text) ? ` — ${one(n.text)}` : ""}`);
  if (files.length) parts.push(`## ${labels.files ?? "references"}\n${files.join("\n")}`);
  const frames = doc.nodes
    .filter((n) => n.kind === "frame")
    .map((f) => {
      const held = doc.nodes.filter((n) => n.kind !== "frame" && n.kind !== "sketch" && inside(n, f)).map((n) => one(n.text) || one(n.name)).filter(Boolean);
      return `- **${one(f.text) || "—"}**${held.length ? `: ${held.join(" · ")}` : ""}`;
    });
  if (frames.length) parts.push(`## ${labels.frames ?? "frames"}\n${frames.join("\n")}`);
  return parts.join("\n\n");
}
