<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { inTauri, bring } from "./ipc.svelte";
  import { store, type DesignNode, type DesignOp, type DesignTag, type Stroke } from "./store.svelte";
  import { strokePath, strokesBox, worldStrokes, localStrokes, overlaps, edgeEnds, TAG_COLORS, type Box } from "./ink";
  import { t } from "./i18n.svelte";

  /**
   * The design's sketch board, after Huabu: notes, freehand ink and arrows,
   * reference files (pictures, PDFs, documents), links, frames that group
   * and questions the human answers, on an endless surface. Files come by
   * dropping them on the board, pasting, or the upload tool. Drag the empty
   * board to pan, wheel to zoom. The agent's changes show dashed until kept
   * or reverted, right on the item. `selected` is what goes with the next
   * message to the agent (picked files go as attachments).
   */
  let { selected = $bindable<string[]>([]) }: { selected?: string[] } = $props();

  type Tool = "select" | "lasso" | "note" | "pen" | "eraser" | "arrow" | "frame" | "question";
  let tool = $state<Tool>("select");
  let penColor = $state("");
  const PEN_COLORS = ["", "#e03127", "#3b82f6", "#46c46a"];

  let el = $state<HTMLDivElement>();
  let view = $state({ x: 0, y: 0, k: 1 });
  const doc = $derived(store.designDoc);

  /** Agent suggestions by the item they are about. */
  const pending = $derived(new Map(doc.changes.map((c) => [c.target, c.id])));

  // ----- coordinates -----
  function toWorld(e: { clientX: number; clientY: number }) {
    const r = el!.getBoundingClientRect();
    return { x: (e.clientX - r.left - view.x) / view.k, y: (e.clientY - r.top - view.y) / view.k };
  }

  /** Fit the board's items in view, once per design opened. */
  let fittedFor = "";
  function fit() {
    if (!el) return;
    const r = el.getBoundingClientRect();
    if (!doc.nodes.length) {
      view = { x: r.width / 2, y: r.height / 3, k: 1 };
      return;
    }
    const minX = Math.min(...doc.nodes.map((n) => n.x));
    const minY = Math.min(...doc.nodes.map((n) => n.y));
    const maxX = Math.max(...doc.nodes.map((n) => n.x + n.w));
    const maxY = Math.max(...doc.nodes.map((n) => n.y + n.h));
    const pad = 80;
    const k = Math.min(1.2, Math.max(0.2, Math.min((r.width - pad * 2) / (maxX - minX || 1), (r.height - pad * 2) / (maxY - minY || 1))));
    view = { k, x: r.width / 2 - ((minX + maxX) / 2) * k, y: r.height / 2 - ((minY + maxY) / 2) * k };
  }
  $effect(() => {
    const id = store.artifact;
    if (id && store.designLoaded === id && fittedFor !== id && el) {
      fittedFor = id;
      queueMicrotask(fit);
    }
  });

  function zoomBy(factor: number, cx?: number, cy?: number) {
    if (!el) return;
    const r = el.getBoundingClientRect();
    const mx = cx ?? r.width / 2;
    const my = cy ?? r.height / 2;
    const k = Math.min(3, Math.max(0.15, view.k * factor));
    view = { k, x: mx - ((mx - view.x) * k) / view.k, y: my - ((my - view.y) * k) / view.k };
  }

  onMount(() => {
    // Not passive: the board takes the wheel instead of the page.
    //  - A touchpad pinch arrives as a wheel with Ctrl held (WebView2's own
    //    pinch zoom is off), in small steps: zoom, briskly. Ctrl+wheel too.
    //  - A mouse wheel (whole notches, no sideways part): zoom.
    //  - Anything else is two fingers on a touchpad: pan, both ways.
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = el!.getBoundingClientRect();
      const cx = e.clientX - r.left;
      const cy = e.clientY - r.top;
      const lines = e.deltaMode === 1;
      if (e.ctrlKey || e.metaKey) {
        zoomBy(Math.exp(-e.deltaY * (lines ? 0.05 : 0.01)), cx, cy);
        return;
      }
      const legacy = (e as WheelEvent & { wheelDeltaY?: number }).wheelDeltaY;
      const notch = legacy !== undefined ? legacy !== 0 && legacy % 120 === 0 : Math.abs(e.deltaY) >= 50 && Number.isInteger(e.deltaY);
      if (e.deltaX === 0 && !e.shiftKey && (lines || notch)) {
        zoomBy(Math.exp(-e.deltaY * (lines ? 0.05 : 0.0015)), cx, cy);
        return;
      }
      const dx = e.shiftKey && e.deltaX === 0 ? e.deltaY : e.deltaX;
      const dy = e.shiftKey && e.deltaX === 0 ? 0 : e.deltaY;
      const k = lines ? 20 : 1;
      view = { ...view, x: view.x - dx * k, y: view.y - dy * k };
    };
    el?.addEventListener("wheel", onWheel, { passive: false });
    return () => el?.removeEventListener("wheel", onWheel);
  });

  // ----- where items are drawn, with a drag or resize in progress -----
  // After the drop they stay drawn where they were dropped (`settle` is the
  // board version then) until a newer board comes back with them there, so
  // nothing snaps back while the core answers.
  let drag = $state<{ dx: number; dy: number; moved: boolean; settle?: number } | null>(null);
  let resize = $state<{ id: string; w: number; h: number; settle?: number } | null>(null);
  const dragNow = $derived(drag && drag.settle !== undefined && doc.version > drag.settle ? null : drag);
  const resizeNow = $derived(resize && resize.settle !== undefined && doc.version > resize.settle ? null : resize);
  $effect(() => {
    const v = doc.version;
    if (drag?.settle !== undefined && v > drag.settle) drag = null;
    if (resize?.settle !== undefined && v > resize.settle) resize = null;
  });

  /** The box being dragged out to pick with. */
  let marquee = $state<Box | null>(null);

  /** What a picking box takes: cards it touches; frames and ink only when wholly inside. */
  function taken(m: Box): string[] {
    const touches = (b: Box) => b.x < m.x + m.w && m.x < b.x + b.w && b.y < m.y + m.h && m.y < b.y + b.h;
    const within = (b: Box) => b.x >= m.x && b.y >= m.y && b.x + b.w <= m.x + m.w && b.y + b.h <= m.y + m.h;
    return doc.nodes.filter((n) => (n.kind === "frame" || n.kind === "sketch" ? within(n) : touches(n))).map((n) => n.id);
  }

  /** The loop being drawn to pick with (the lasso), in board coordinates. */
  let lasso = $state<{ x: number; y: number }[] | null>(null);

  /** Whether a point is inside a closed loop (even-odd rule). */
  function inLoop(px: number, py: number, loop: { x: number; y: number }[]): boolean {
    let inside = false;
    for (let i = 0, j = loop.length - 1; i < loop.length; j = i++) {
      const a = loop[i];
      const b = loop[j];
      if (a.y > py !== b.y > py && px < ((b.x - a.x) * (py - a.y)) / (b.y - a.y) + a.x) inside = !inside;
    }
    return inside;
  }

  /** What a loop takes, by the box's rule: a card it reaches into (a corner
   *  or the middle inside, or the loop passing through the card); frames and
   *  ink only when wholly inside. */
  function looped(loop: { x: number; y: number }[]): string[] {
    if (loop.length < 3) return [];
    const corners = (b: Box) => [
      [b.x, b.y],
      [b.x + b.w, b.y],
      [b.x, b.y + b.h],
      [b.x + b.w, b.y + b.h],
    ];
    const within = (b: Box) => corners(b).every(([x, y]) => inLoop(x, y, loop));
    const touches = (b: Box) =>
      inLoop(b.x + b.w / 2, b.y + b.h / 2, loop) ||
      corners(b).some(([x, y]) => inLoop(x, y, loop)) ||
      loop.some((p) => p.x >= b.x && p.x <= b.x + b.w && p.y >= b.y && p.y <= b.y + b.h);
    return doc.nodes.filter((n) => (n.kind === "frame" || n.kind === "sketch" ? within(n) : touches(n))).map((n) => n.id);
  }

  /** What a drag moves: the picked items and what picked frames hold. */
  let dragIds = $state<string[]>([]);

  function inside(n: DesignNode, f: DesignNode): boolean {
    return n.x >= f.x && n.y >= f.y && n.x + n.w <= f.x + f.w && n.y + n.h <= f.y + f.h;
  }

  function withMembers(ids: string[]): string[] {
    const out = new Set(ids);
    for (const f of doc.nodes) {
      if (f.kind !== "frame" || !ids.includes(f.id)) continue;
      for (const n of doc.nodes) if (n.kind !== "frame" && inside(n, f)) out.add(n.id);
    }
    return [...out];
  }

  function box(n: DesignNode): Box {
    let { x, y, w, h } = n;
    if (dragNow && dragIds.includes(n.id)) {
      x += dragNow.dx;
      y += dragNow.dy;
    }
    if (resizeNow && resizeNow.id === n.id) {
      w = resizeNow.w;
      h = resizeNow.h;
    }
    return { x, y, w, h };
  }

  const boxes = $derived(new Map(doc.nodes.map((n) => [n.id, box(n)])));

  /** The svg that holds arrows covers every item, so lines stay clickable. */
  const edgeFrame = $derived.by(() => {
    const all = [...boxes.values()];
    if (!all.length) return { x: 0, y: 0, w: 1, h: 1 };
    const minX = Math.min(...all.map((b) => b.x)) - 60;
    const minY = Math.min(...all.map((b) => b.y)) - 60;
    const maxX = Math.max(...all.map((b) => b.x + b.w)) + 60;
    const maxY = Math.max(...all.map((b) => b.y + b.h)) + 60;
    return { x: minX, y: minY, w: maxX - minX, h: maxY - minY };
  });

  // ----- ink -----
  let ink = $state.raw<Stroke | null>(null);
  /** Strokes crossed out by the eraser, by sketch id, until the pointer lifts. */
  let erased = $state<Record<string, number[]>>({});

  /** Hand a finished stroke to the core, which merges it into the sketch it
   *  touches; strokes drawn in quick succession never overwrite each other. */
  async function commitInk(stroke: Stroke) {
    await store.designApply([{ op: "add_stroke", stroke }]);
    // A newer stroke may already be under way; only this one is done.
    if (ink === stroke) ink = null;
  }

  /** Stroke outlines are costly; each is kept until its stroke changes. */
  const outlines = new Map<string, string>();
  function outline(n: DesignNode, i: number, s: Stroke): string {
    const p = s.points[0];
    const key = `${n.id}:${i}:${s.points.length}:${p?.[0]}:${p?.[1]}:${s.size}`;
    let d = outlines.get(key);
    if (d === undefined) {
      if (outlines.size > 20000) outlines.clear();
      d = strokePath(s);
      outlines.set(key, d);
    }
    return d;
  }

  function eraseAt(p: { x: number; y: number }) {
    const r = 10 / view.k;
    for (const n of doc.nodes) {
      if (n.kind !== "sketch" || !overlaps(n, { x: p.x - r, y: p.y - r, w: r * 2, h: r * 2 })) continue;
      n.strokes.forEach((s, i) => {
        if ((erased[n.id] ?? []).includes(i)) return;
        const hit = s.points.some(([x, y]) => (x + n.x - p.x) ** 2 + (y + n.y - p.y) ** 2 < r * r);
        if (hit) erased = { ...erased, [n.id]: [...(erased[n.id] ?? []), i] };
      });
    }
  }

  async function commitErase() {
    const ops: DesignOp[] = [];
    for (const [id, gone] of Object.entries(erased)) {
      const n = doc.nodes.find((x) => x.id === id);
      if (!n) continue;
      const keep = worldStrokes(n).filter((_, i) => !gone.includes(i));
      if (!keep.length) ops.push({ op: "set_sketch", id, x: n.x, y: n.y, w: n.w, h: n.h, strokes: [] });
      else {
        const nb = strokesBox(keep);
        ops.push({ op: "set_sketch", id, ...nb, strokes: localStrokes(keep, nb) });
      }
    }
    await store.designApply(ops);
    erased = {};
  }

  // ----- editing a note -----
  let editing = $state("");
  let editText = $state("");
  /** A new note to edit as soon as the core's board has it. */
  let pendingEdit = $state("");
  $effect(() => {
    const ids = new Set(doc.nodes.map((n) => n.id));
    if (pendingEdit && ids.has(pendingEdit)) {
      editing = pendingEdit;
      editText = "";
      pendingEdit = "";
    }
    // Removed under our feet (the agent, a revert): nothing to edit or pick.
    if (editing && !ids.has(editing)) editing = "";
    if (selected.some((id) => !ids.has(id))) selected = selected.filter((id) => ids.has(id));
    if (selectedEdge && !doc.edges.some((e) => e.id === selectedEdge)) selectedEdge = "";
    if (arrowFrom && !ids.has(arrowFrom)) arrowFrom = "";
  });
  function startEdit(n: DesignNode) {
    if (n.kind === "sketch") return;
    editing = n.id;
    editText = n.text;
  }

  /** The question whose answer is being written. */
  let answering = $state("");
  let answerText = $state("");
  function startAnswer(n: DesignNode) {
    answering = n.id;
    answerText = n.answer ?? "";
  }
  async function endAnswer() {
    const id = answering;
    if (!id) return;
    answering = "";
    const n = doc.nodes.find((x) => x.id === id);
    if (n && (n.answer ?? "") !== answerText) await store.designApply([{ op: "update", id, answer: answerText }]);
  }

  // ----- frames, questions, links, files -----
  /** A frame being drawn. */
  let framing = $state<Box | null>(null);
  /** Where a link is being typed in, and what. */
  let linkAt = $state<{ x: number; y: number } | null>(null);
  let linkDraft = $state("");
  /** A drop from the system is over the board. */
  let dropOver = $state(false);

  /** The middle of the view, in board units. */
  function centre(): { x: number; y: number } {
    const r = el!.getBoundingClientRect();
    return { x: (r.width / 2 - view.x) / view.k, y: (r.height / 2 - view.y) / view.k };
  }

  async function addLink() {
    const url = linkDraft.trim();
    const at = linkAt;
    linkAt = null;
    linkDraft = "";
    if (!url || !at) return;
    const href = /^https?:\/\//i.test(url) ? url : `https://${url}`;
    await store.designApply([{ op: "create_link", x: at.x, y: at.y, url: href }]);
  }

  function hostOf(url: string | undefined): string {
    try {
      return new URL(url ?? "").host;
    } catch {
      return url ?? "";
    }
  }

  function extOf(name: string | undefined): string {
    const m = /\.([a-z0-9]+)$/i.exec(name ?? "");
    return m ? m[1].toUpperCase() : "FILE";
  }

  onMount(() => {
    store.designBoardEl = el ?? null;
    // Files dropped from the system onto the board become cards where they land.
    let unlisten: (() => void) | undefined;
    let gone = false;
    // Files dropped from the system: only in the app's own window.
    if (inTauri) getCurrentWebview()
      .onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === "leave") {
          dropOver = false;
          return;
        }
        const over = store.overDesignBoard(p.position);
        if (p.type === "enter" || p.type === "over") dropOver = over;
        else if (p.type === "drop") {
          dropOver = false;
          if (!over || !el) return;
          const at = toWorld({ clientX: p.position.x / devicePixelRatio, clientY: p.position.y / devicePixelRatio });
          void bring(p.paths)
            .then((paths) => store.designAddFiles(paths, Math.round(at.x), Math.round(at.y)))
            .catch((err) => (store.lastError = String(err)));
        }
      })
      .then((u) => (gone ? u() : (unlisten = u)))
      .catch(() => {});
    // Pasting onto the board: a picture becomes a file card, an address a
    // link, any other text a note.
    const onPaste = (e: ClipboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (store.view !== "design" || editing || answering || linkAt || store.tagDialog) return;
      if (target?.closest?.("input, textarea, select, [contenteditable], [role=menu], [role=dialog]")) return;
      if (target && target !== document.body && !el?.contains(target)) return;
      const data = e.clipboardData;
      if (!data || !el) return;
      const at = centre();
      const files = [...data.files];
      if (files.length) {
        e.preventDefault();
        files.forEach((f, i) => {
          const ext = f.type.split("/")[1]?.replace("jpeg", "jpg") ?? "bin";
          void store.designAddBlob(f, f.name || `pasted.${ext}`, Math.round(at.x + i * 40), Math.round(at.y + i * 40));
        });
        return;
      }
      const text = data.getData("text/plain").trim();
      if (!text) return;
      e.preventDefault();
      if (/^https?:\/\/\S+$/i.test(text)) void store.designApply([{ op: "create_link", x: Math.round(at.x), y: Math.round(at.y), url: text }]);
      else void store.designApply([{ op: "create_note", x: Math.round(at.x - 130), y: Math.round(at.y - 75), text }]);
    };
    window.addEventListener("paste", onPaste);
    return () => {
      gone = true;
      unlisten?.();
      window.removeEventListener("paste", onPaste);
      if (store.designBoardEl === el) store.designBoardEl = null;
    };
  });
  async function endEdit() {
    const id = editing;
    if (!id) return;
    editing = "";
    const n = doc.nodes.find((x) => x.id === id);
    if (n && n.text !== editText) await store.designApply([{ op: "update", id, text: editText }]);
  }

  // ----- arrows -----
  let arrowFrom = $state("");
  let selectedEdge = $state("");

  // ----- pointer -----
  /** Space is down: a press on the board pans it (as in Huabu and Figma). */
  let spaceHeld = $state(false);
  /** A pan in progress, for the closed hand. */
  let panning = $state(false);

  /** Space down or up, wherever the focus is: after typing in the chat, the
   *  space still reaches here, so space + click pans at once. It is only
   *  kept from typing when the board itself has the focus. */
  function onSpace(e: KeyboardEvent, down: boolean) {
    if (e.key !== " ") return;
    if (!down) {
      spaceHeld = false;
      return;
    }
    if (store.view !== "design" || editing) return;
    spaceHeld = true;
    // On the board (not in a text box) a space would press the focused button or scroll.
    const target = e.target as HTMLElement | null;
    if (el && target && (target === el || el.contains(target) || target === document.body) && !target.closest?.("input, textarea, [contenteditable]")) {
      e.preventDefault();
    }
  }

  /** A press on the board takes the focus from the chat box or a card's
   *  viewer, so keys (space, tools, undo) are the board's; not while a card
   *  is being typed in. */
  function takeFocus(e: PointerEvent) {
    const target = e.target as HTMLElement | null;
    if (!el || editing || target?.closest?.("input, textarea, [contenteditable]")) return;
    if (document.activeElement !== el) el.focus({ preventScroll: true });
  }

  function onDown(e: PointerEvent) {
    if (!el || editing) return;
    const target = e.target as Element;
    const nodeEl = target.closest<HTMLElement>("[data-node]");
    // A frame's open area is board to pick in; the frame itself is taken by its title.
    const frameBody = tool === "select" && !!nodeEl?.classList.contains("frame") && !target.closest(".ftitle");
    const id = frameBody ? "" : (nodeEl?.dataset.node ?? "");
    const p = toWorld(e);

    // Pan: the middle button, or space held (a touchpad pans with two fingers).
    if (e.button === 1 || spaceHeld) {
      e.preventDefault();
      const start = { cx: e.clientX, cy: e.clientY, x: view.x, y: view.y };
      panning = true;
      follow(
        e,
        (ev) => (view = { ...view, x: start.x + ev.clientX - start.cx, y: start.y + ev.clientY - start.cy }),
        () => (panning = false),
        () => (panning = false),
      );
      return;
    }
    if (e.button !== 0) return;

    // The lasso (O, or Alt with the select tool on the empty board): draw a
    // loop around what to pick (Shift adds to what is picked). Over cards
    // too; the select tool comes back once it is drawn, to move them.
    if (tool === "lasso" || (tool === "select" && !id && e.altKey)) {
      e.preventDefault();
      const keep = e.shiftKey ? [...selected] : [];
      if (!e.shiftKey) {
        selected = [];
        selectedEdge = "";
      }
      lasso = [p];
      follow(
        e,
        (ev) => {
          const q = toWorld(ev);
          const last = lasso?.at(-1);
          // A point every few pixels on screen is plenty.
          if (!lasso || (last && Math.hypot(q.x - last.x, q.y - last.y) * view.k < 3)) return;
          lasso = [...lasso, q];
          selected = [...new Set([...keep, ...looped(lasso)])];
        },
        () => {
          lasso = null;
          tool = "select";
        },
        () => (lasso = null),
      );
      return;
    }

    // The empty board with the select tool: drag a box to pick what it
    // takes in (Shift adds to what is picked); a click clears.
    if (tool === "select" && !id) {
      const start = p;
      const keep = e.shiftKey ? [...selected] : [];
      if (!e.shiftKey) {
        selected = [];
        selectedEdge = "";
      }
      marquee = { x: p.x, y: p.y, w: 0, h: 0 };
      follow(
        e,
        (ev) => {
          const q = toWorld(ev);
          const m = { x: Math.min(start.x, q.x), y: Math.min(start.y, q.y), w: Math.abs(q.x - start.x), h: Math.abs(q.y - start.y) };
          marquee = m;
          selected = [...new Set([...keep, ...taken(m)])];
        },
        () => (marquee = null),
        () => (marquee = null),
      );
      return;
    }

    if (tool === "question") {
      tool = "select";
      void store.designApply([{ op: "create_question", x: p.x - 130, y: p.y - 95, text: "" }]).then((res) => {
        const nid = res[0]?.id;
        if (nid) {
          selected = [nid];
          pendingEdit = nid;
        }
      });
      return;
    }

    if (tool === "frame") {
      const start = p;
      framing = { x: p.x, y: p.y, w: 0, h: 0 };
      follow(
        e,
        (ev) => {
          const q = toWorld(ev);
          framing = { x: Math.min(start.x, q.x), y: Math.min(start.y, q.y), w: Math.abs(q.x - start.x), h: Math.abs(q.y - start.y) };
        },
        () => {
          const f = framing;
          framing = null;
          tool = "select";
          if (!f) return;
          // A click rather than a drag: a frame of the usual size.
          const b = f.w < 40 || f.h < 40 ? { x: f.x, y: f.y, w: 640, h: 420 } : f;
          void store
            .designApply([{ op: "create_frame", x: Math.round(b.x), y: Math.round(b.y), w: Math.round(b.w), h: Math.round(b.h), title: "" }])
            .then((res) => {
              const nid = res[0]?.id;
              if (nid) {
                selected = [nid];
                pendingEdit = nid;
              }
            });
        },
        () => (framing = null),
      );
      return;
    }

    if (tool === "note") {
      tool = "select";
      void store.designApply([{ op: "create_note", x: p.x - 130, y: p.y - 75, text: "" }]).then((res) => {
        const nid = res[0]?.id;
        if (nid) {
          selected = [nid];
          pendingEdit = nid;
        }
      });
      return;
    }

    if (tool === "pen") {
      const pressure = e.pressure > 0 && e.pointerType === "pen" ? e.pressure : 0.5;
      ink = { points: [[p.x, p.y, pressure]], color: penColor, size: 4 };
      follow(
        e,
        (ev) => {
          const q = toWorld(ev);
          const pr = ev.pressure > 0 && ev.pointerType === "pen" ? ev.pressure : 0.5;
          if (ink) ink = { ...ink, points: [...ink.points, [q.x, q.y, pr]] };
        },
        () => {
          if (ink && ink.points.length > 1) void commitInk(ink);
          else ink = null;
        },
        () => (ink = null),
      );
      return;
    }

    if (tool === "eraser") {
      eraseAt(p);
      follow(
        e,
        (ev) => eraseAt(toWorld(ev)),
        () => void commitErase(),
        () => (erased = {}),
      );
      return;
    }

    if (tool === "arrow") {
      if (!id) {
        arrowFrom = "";
        return;
      }
      if (!arrowFrom) arrowFrom = id;
      else if (arrowFrom !== id) {
        const from = arrowFrom;
        arrowFrom = "";
        void store.designApply([{ op: "connect", from, to: id }]);
      }
      return;
    }

    // Select tool on an item: pick it and drag.
    selectedEdge = "";
    if (e.shiftKey) selected = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id];
    else if (!selected.includes(id)) selected = [id];
    if (!selected.includes(id)) return;
    const start = p;
    dragIds = withMembers(selected);
    drag = { dx: 0, dy: 0, moved: false };
    follow(
      e,
      (ev) => {
        const q = toWorld(ev);
        const dx = q.x - start.x;
        const dy = q.y - start.y;
        drag = { dx, dy, moved: !!drag?.moved || Math.hypot(dx, dy) > 3 / view.k };
      },
      () => {
        const d = drag;
        if (!d?.moved) {
          drag = null;
          return;
        }
        const ops: DesignOp[] = doc.nodes
          .filter((n) => dragIds.includes(n.id))
          .map((n) => ({ op: "move", id: n.id, x: Math.round(n.x + d.dx), y: Math.round(n.y + d.dy) }));
        drag = { ...d, settle: doc.version };
        void store.designApply(ops).then((res) => {
          if (!res.length || res.some((r) => !r.ok)) drag = null;
        });
      },
      () => (drag = null),
    );
  }

  function startResize(e: PointerEvent, n: DesignNode) {
    e.stopPropagation();
    const start = toWorld(e);
    resize = { id: n.id, w: n.w, h: n.h };
    follow(
      e,
      (ev) => {
        const q = toWorld(ev);
        resize = { id: n.id, w: Math.max(120, n.w + q.x - start.x), h: Math.max(60, n.h + q.y - start.y) };
      },
      () => {
        const r = resize;
        if (!r) return;
        resize = { ...r, settle: doc.version };
        void store.designApply([{ op: "move", id: n.id, x: n.x, y: n.y, w: Math.round(r.w), h: Math.round(r.h) }]).then((res) => {
          if (!res.length || res.some((x) => !x.ok)) resize = null;
        });
      },
      () => (resize = null),
    );
  }

  /** The gesture under way, so leaving the board mid-gesture ends it. */
  let stopFollowing: (() => void) | null = null;
  onDestroy(() => stopFollowing?.());

  /** Track the pointer that started a gesture until it lifts. Other pointers
   *  (a second finger) are ignored; a cancelled pointer (a pen out of range)
   *  ends the gesture without its effect. */
  function follow(start: PointerEvent, move: (e: PointerEvent) => void, up: () => void, cancel: () => void = () => {}) {
    stopFollowing?.();
    const id = start.pointerId;
    const onMove = (ev: PointerEvent) => {
      if (ev.pointerId === id) move(ev);
    };
    const stop = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      stopFollowing = null;
    };
    const onUp = (ev: PointerEvent) => {
      if (ev.pointerId !== id) return;
      stop();
      up();
    };
    const onCancel = (ev: PointerEvent) => {
      if (ev.pointerId !== id) return;
      stop();
      cancel();
    };
    stopFollowing = () => {
      stop();
      cancel();
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
  }

  function setTag(tag: DesignTag) {
    const ops: DesignOp[] = selected
      .filter((id) => doc.nodes.find((n) => n.id === id)?.kind === "note")
      .map((id) => ({ op: "update", id, tag }));
    void store.designApply(ops);
  }

  function removeSelected() {
    const ids = [...selected, ...(selectedEdge ? [selectedEdge] : [])];
    if (!ids.length) return;
    selected = [];
    selectedEdge = "";
    void store.designApply([{ op: "delete", ids }]);
  }

  function onKey(e: KeyboardEvent) {
    // Not while typing, nor behind a dialog or menu (the tag dialog, a
    // right-click menu), nor on another view.
    const target = e.target as HTMLElement | null;
    if (e.defaultPrevented || editing || store.tagDialog || store.view !== "design") return;
    if (target?.closest?.("input, textarea, select, [contenteditable], [role=menu], [role=dialog]")) return;
    // Undo and redo of the human's own edits (typing has its own).
    if ((e.ctrlKey || e.metaKey) && !e.altKey) {
      const k = e.key.toLowerCase();
      if (k === "z" || k === "y") {
        e.preventDefault();
        void store.designUndo(k === "y" || e.shiftKey);
      }
      return;
    }
    if (e.altKey) return;
    const keys: Record<string, Tool> = { v: "select", o: "lasso", n: "note", p: "pen", e: "eraser", a: "arrow", f: "frame", q: "question" };
    const k = e.key.toLowerCase();
    if (k === "l" && el) {
      e.preventDefault();
      linkAt = centre();
    } else if (k === "u" && el) {
      e.preventDefault();
      const c = centre();
      void store.designPickFiles(Math.round(c.x), Math.round(c.y));
    } else if (keys[k]) {
      tool = keys[k];
      arrowFrom = "";
    } else if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      removeSelected();
    } else if (e.key === "Escape") {
      selected = [];
      selectedEdge = "";
      arrowFrom = "";
      tool = "select";
    }
  }

  const single = $derived(selected.length === 1 ? doc.nodes.find((n) => n.id === selected[0]) : undefined);
  const TAG_LIST: DesignTag[] = ["goal", "constraint", "question", "idea"];

  function focusOnMount(node: HTMLTextAreaElement | HTMLInputElement) {
    node.focus();
    node.setSelectionRange(node.value.length, node.value.length);
  }
</script>

<svelte:window
  onkeydowncapture={(e) => onSpace(e, true)}
  onkeyupcapture={(e) => onSpace(e, false)}
  onkeydown={onKey}
  onblur={() => {
    spaceHeld = false;
    panning = false;
  }}
/>

<div
  class="board"
  class:pen={tool === "pen"}
  class:eraser={tool === "eraser"}
  class:note={tool === "note"}
  class:arrow={tool === "arrow"}
  class:placing={tool === "frame" || tool === "question"}
  class:dropover={dropOver}
  bind:this={el}
  class:grab={spaceHeld}
  class:grabbing={panning}
  tabindex="-1"
  onpointerdowncapture={takeFocus}
  onpointerdown={onDown}
  role="application"
  aria-label={t("design.board")}
  style="background-position: {view.x}px {view.y}px; background-size: {22 * view.k}px {22 * view.k}px"
>
  <div class="world" style="transform: translate({view.x}px, {view.y}px) scale({view.k})">
    <!-- Arrows under the items. -->
    <svg class="edges" style="left: {edgeFrame.x}px; top: {edgeFrame.y}px" width={edgeFrame.w} height={edgeFrame.h}>
      <defs>
        <marker id="design-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M0,0 L10,5 L0,10 z" fill="context-stroke" />
        </marker>
      </defs>
      {#each doc.edges as e (e.id)}
        {@const a = boxes.get(e.from)}
        {@const b = boxes.get(e.to)}
        {#if a && b}
          {@const [x1, y1, x2, y2] = edgeEnds(a, b)}
          <g class="edge" class:pending={pending.has(e.id)} class:on={selectedEdge === e.id}>
            <line
              class="hit"
              x1={x1 - edgeFrame.x}
              y1={y1 - edgeFrame.y}
              x2={x2 - edgeFrame.x}
              y2={y2 - edgeFrame.y}
              role="presentation"
              onpointerdown={(ev) => {
                if (tool !== "select") return;
                ev.stopPropagation();
                selectedEdge = e.id;
                selected = [];
              }}
            />
            <line class="line" x1={x1 - edgeFrame.x} y1={y1 - edgeFrame.y} x2={x2 - edgeFrame.x} y2={y2 - edgeFrame.y} marker-end="url(#design-arrow)" />
          </g>
        {/if}
      {/each}
    </svg>

    {#each doc.nodes as n (n.id)}
      {@const b = boxes.get(n.id) ?? n}
      {@const change = pending.get(n.id)}
      <div
        class="node {n.kind}"
        class:on={selected.includes(n.id)}
        class:pending={change !== undefined}
        class:from={arrowFrom === n.id}
        data-node={n.id}
        style="left: {b.x}px; top: {b.y}px; width: {b.w}px; height: {b.h}px; {n.tag ? `--tag: ${TAG_COLORS[n.tag]}` : ''}"
        ondblclick={() => startEdit(n)}
        role="presentation"
      >
        {#if n.kind === "frame"}
          <div class="ftitle">
            {#if editing === n.id}
              <input
                class="fedit"
                bind:value={editText}
                onblur={endEdit}
                onpointerdown={(ev) => ev.stopPropagation()}
                onkeydown={(ev) => (ev.key === "Enter" || ev.key === "Escape") && (ev.currentTarget as HTMLInputElement).blur()}
                use:focusOnMount
              />
            {:else}
              <span class:empty={!n.text}>{n.text || t("design.frameUntitled")}</span>
            {/if}
          </div>
        {:else if n.kind === "file"}
          {@const url = store.boardFileUrl(n.src ?? "")}
          {#if (n.mime ?? "").startsWith("image/")}
            <img class="pic" src={url} alt={n.name} draggable="false" />
          {:else}
            <div class="fhead">
              <span class="mono ext">{extOf(n.name)}</span>
              <span class="fname" title={n.name}>{n.name}</span>
              {@render readState(n.id)}
              <button type="button" class="fopen" title={t("design.open")} onpointerdown={(ev) => ev.stopPropagation()} onclick={() => store.designOpenFile(n.src ?? "")}>↗</button>
            </div>
            {#if n.mime === "application/pdf"}
              <!-- Scrolls and zooms only once picked, so the card still drags. -->
              <iframe class="pdf" class:live={single?.id === n.id} src={url} title={n.name}></iframe>
            {/if}
          {/if}
          {#if editing === n.id}
            <textarea class="edit caption" bind:value={editText} onblur={endEdit} onpointerdown={(ev) => ev.stopPropagation()} use:focusOnMount></textarea>
          {:else if n.text}
            <div class="caption">{n.text}</div>
          {/if}
        {:else if n.kind === "link"}
          <div class="lhead">
            <span class="ltitle">{n.name || hostOf(n.url)}</span>
            {@render readState(n.id)}
            <button type="button" class="fopen" title={t("design.openLink")} onpointerdown={(ev) => ev.stopPropagation()} onclick={() => store.openUrl(n.url ?? "")}>↗</button>
          </div>
          <div class="mono lurl" title={n.url}>{n.url}</div>
          {#if editing === n.id}
            <textarea class="edit" bind:value={editText} onblur={endEdit} onpointerdown={(ev) => ev.stopPropagation()} use:focusOnMount></textarea>
          {:else if n.text}
            <div class="text">{n.text}</div>
          {/if}
        {:else if n.kind === "question"}
          <span class="mono qlab">? {t("design.questionLabel")}</span>
          {#if editing === n.id}
            <textarea class="edit" bind:value={editText} onblur={endEdit} onpointerdown={(ev) => ev.stopPropagation()} use:focusOnMount></textarea>
          {:else}
            <div class="text" class:empty={!n.text}>{n.text || t("design.emptyNote")}</div>
          {/if}
          <div class="answer" role="presentation" onpointerdown={(ev) => ev.stopPropagation()} ondblclick={(ev) => ev.stopPropagation()} onclick={() => answering !== n.id && startAnswer(n)}>
            {#if answering === n.id}
              <textarea
                class="edit"
                bind:value={answerText}
                placeholder={t("design.answerPlaceholder")}
                onblur={endAnswer}
                onkeydown={(ev) => (ev.key === "Escape" || (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey))) && (ev.currentTarget as HTMLTextAreaElement).blur()}
                use:focusOnMount
              ></textarea>
            {:else}
              <span class:empty={!n.answer}>{n.answer || t("design.answerPlaceholder")}</span>
            {/if}
          </div>
        {:else if n.kind === "note"}
          {#if n.tag}<span class="mono tag">{t(`design.tag.${n.tag}` as "design.tag.goal")}</span>{/if}
          {#if editing === n.id}
            <textarea
              class="edit"
              bind:value={editText}
              onblur={endEdit}
              onpointerdown={(ev) => ev.stopPropagation()}
              onkeydown={(ev) => {
                if (ev.key === "Escape" || (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey))) {
                  ev.preventDefault();
                  (ev.currentTarget as HTMLTextAreaElement).blur();
                }
              }}
              use:focusOnMount
            ></textarea>
          {:else}
            <div class="text" class:empty={!n.text}>{n.text || t("design.emptyNote")}</div>
          {/if}
        {:else}
          <svg class="ink" width={b.w} height={b.h} viewBox="0 0 {n.w} {n.h}" preserveAspectRatio="none">
            {#each n.strokes as s, i (i)}
              <path d={outline(n, i, s)} fill={s.color || "var(--txt)"} class:gone={(erased[n.id] ?? []).includes(i)} />
            {/each}
          </svg>
        {/if}
        {#if n.kind !== "sketch" && selected.length === 1 && selected[0] === n.id && editing !== n.id}
          <span class="grip" role="presentation" onpointerdown={(ev) => startResize(ev, n)}></span>
        {/if}
        {#if change !== undefined}
          <!-- The agent's suggestion: keep it or put it back, right here. -->
          <span class="review" role="presentation" onpointerdown={(ev) => ev.stopPropagation()}>
            <span class="mono sug">{t("design.suggested")}</span>
            <button type="button" onclick={() => store.designReview([change], true)} title={t("design.keep")} aria-label={t("design.keep")}>✓</button>
            <button type="button" onclick={() => store.designReview([change], false)} title={t("design.revert")} aria-label={t("design.revert")}>↺</button>
          </span>
        {/if}
      </div>
    {/each}

    {#if framing}
      <div class="framing" style="left: {framing.x}px; top: {framing.y}px; width: {framing.w}px; height: {framing.h}px"></div>
    {/if}
    {#if marquee && (marquee.w > 2 || marquee.h > 2)}
      <div class="marquee" style="left: {marquee.x}px; top: {marquee.y}px; width: {marquee.w}px; height: {marquee.h}px"></div>
    {/if}
    {#if linkAt}
      <input
        class="linkin"
        style="left: {linkAt.x - 160}px; top: {linkAt.y - 18}px"
        placeholder={t("design.linkPlaceholder")}
        bind:value={linkDraft}
        onpointerdown={(ev) => ev.stopPropagation()}
        onkeydown={(ev) => {
          if (ev.key === "Enter") void addLink();
          else if (ev.key === "Escape") linkAt = null;
        }}
        onblur={() => void addLink()}
        use:focusOnMount
      />
    {/if}

    {#if lasso && lasso.length > 1}
      <svg class="live" style="left: 0; top: 0" width="1" height="1" aria-hidden="true">
        <path class="lassoline" d={`M${lasso.map((q) => `${q.x} ${q.y}`).join(" L")} Z`} vector-effect="non-scaling-stroke" />
      </svg>
    {/if}
    {#if ink}
      <svg class="live" style="left: 0; top: 0" width="1" height="1">
        <path d={strokePath(ink)} fill={ink.color || "var(--txt)"} />
      </svg>
    {/if}
  </div>

  <!-- Tools, like a sketchbook's: V select · O lasso · N note · P pen · E eraser · A arrow. -->
  <div class="tools" role="toolbar" aria-label={t("design.tools")} tabindex="-1" onpointerdown={(e) => e.stopPropagation()}>
    {#each [["select", "V"], ["lasso", "O"], ["note", "N"], ["question", "Q"], ["frame", "F"], ["pen", "P"], ["eraser", "E"], ["arrow", "A"]] as [id, key] (id)}
      <button
        type="button"
        class="tool"
        class:on={tool === id}
        onclick={() => {
          tool = id as Tool;
          arrowFrom = "";
        }}
        title="{t(`design.tool.${id}` as 'design.tool.select')} ({key})"
        aria-label={t(`design.tool.${id}` as "design.tool.select")}
      >
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" aria-hidden="true">
          {#if id === "select"}<path d="M3 2l9 5-4 1-2 4z" />
          {:else if id === "lasso"}<path d="M8 3c3.3 0 5.5 1.6 5.5 3.6S11.3 10 8 10 2.5 8.6 2.5 6.6 4.7 3 8 3z" stroke-dasharray="2 1.6" /><path d="M5 9.5c-.6 1-.6 2.2.2 3" />
          {:else if id === "note"}<rect x="2.5" y="2.5" width="11" height="11" /><path d="M5 6h6M5 9h4" />
          {:else if id === "pen"}<path d="M3 13l1-3 7-7 2 2-7 7z" /><path d="M9.5 4.5l2 2" />
          {:else if id === "eraser"}<path d="M6 13h7M2.5 9.5l5-5 4 4-4 4H5z" />
          {:else if id === "question"}<circle cx="8" cy="8" r="5.5" /><path d="M6.3 6.4a1.8 1.8 0 1 1 2.4 1.7c-.5.2-.7.6-.7 1.1v.4M8 11.5v.3" />
          {:else if id === "frame"}<path d="M2.5 5h11M5 2.5v11M2.5 11h11M11 2.5v11" />
          {:else}<path d="M2 12L13 3M13 3H8M13 3v5" />{/if}
        </svg>
      </button>
    {/each}
    <span class="sep"></span>
    <button type="button" class="tool" title="{t('design.tool.link')} (L)" aria-label={t("design.tool.link")} onclick={() => (linkAt = centre())}>
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" aria-hidden="true"><path d="M6.5 9.5l3-3M7 4.5l1.3-1.3a2.5 2.5 0 0 1 3.5 3.5L10.5 8M9 11.5l-1.3 1.3a2.5 2.5 0 0 1-3.5-3.5L5.5 8" /></svg>
    </button>
    <button
      type="button"
      class="tool"
      title="{t('design.tool.upload')} (U)"
      aria-label={t("design.tool.upload")}
      onclick={() => {
        const c = centre();
        void store.designPickFiles(Math.round(c.x), Math.round(c.y));
      }}
    >
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" aria-hidden="true"><path d="M8 10.5V2.5M5 5.5l3-3 3 3M2.5 10v3.5h11V10" /></svg>
    </button>
    {#if tool === "pen"}
      <span class="sep"></span>
      {#each PEN_COLORS as c (c)}
        <button type="button" class="swatch" class:on={penColor === c} style="--c: {c || 'var(--txt)'}" onclick={() => (penColor = c)} aria-label={c || t("design.ink")}></button>
      {/each}
    {/if}
  </div>

  {#if single?.kind === "note" && !editing}
    <!-- What the note is to the design. -->
    <div class="tagbar" onpointerdown={(e) => e.stopPropagation()} role="toolbar" tabindex="-1" aria-label={t("design.tagAs")}>
      <span class="mono lab">{t("design.tagAs")}</span>
      {#each TAG_LIST as tag (tag)}
        <button type="button" class="tagbtn" class:on={single.tag === tag} style="--tag: {TAG_COLORS[tag]}" onclick={() => setTag(single.tag === tag ? "" : tag)}>
          {t(`design.tag.${tag}` as "design.tag.goal")}
        </button>
      {/each}
    </div>
  {/if}

  <div class="zoom" role="toolbar" tabindex="-1" aria-label={t("design.zoom")} onpointerdown={(e) => e.stopPropagation()}>
    <button type="button" onclick={() => zoomBy(1 / 1.2)} aria-label={t("design.zoomOut")}>−</button>
    <button type="button" class="mono pct" onclick={fit} title={t("design.fit")}>{Math.round(view.k * 100)}%</button>
    <button type="button" onclick={() => zoomBy(1.2)} aria-label={t("design.zoomIn")}>+</button>
  </div>

  {#if !doc.nodes.length}
    <div class="hint">
      <p class="serif">{t("design.emptyTitle")}</p>
      <p class="mono">{t("design.emptyHint")}</p>
    </div>
  {/if}
  {#if tool === "arrow"}
    <div class="mono mode">{arrowFrom ? t("design.arrowTo") : t("design.arrowFrom")}</div>
  {/if}
  {#if dropOver}
    <div class="mono dropnote">{t("design.dropHere")}</div>
  {/if}
</div>

<!-- Whether the agent can read a link's page or a document's text. -->
{#snippet readState(id: string)}
  {@const x = store.designExtracts[id]}
  {#if x?.state === "done"}
    <span class="mono rstate ok" title={t("design.textReady")}>{t("design.textReadyShort")}</span>
  {:else if x?.state === "reading"}
    <span class="mono rstate" title={t("design.textReading")}><span class="dot pulse"></span></span>
  {:else if x?.state === "failed"}
    <button type="button" class="mono rstate bad" title={`${x.error ?? ""}\n${t("design.textRetry")}`} onpointerdown={(ev) => ev.stopPropagation()} onclick={() => store.retryExtract(id)}>{t("design.textFailedShort")}</button>
  {/if}
{/snippet}

<style>
  .rstate {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 9.5px;
    letter-spacing: 0.04em;
    color: var(--lab);
    background: none;
    border: 1px solid transparent;
    padding: 0 4px;
  }

  .rstate.ok {
    color: var(--ok);
  }

  .rstate.bad {
    color: var(--warn);
    border-color: var(--warnln);
    cursor: pointer;
  }

  .board:focus {
    outline: none;
  }

  /* Space held: the board is a sheet to drag; cards' viewers let go of the press. */
  .board.grab,
  .board.grab :global(*) {
    cursor: grab !important;
  }

  .board.grabbing,
  .board.grabbing :global(*) {
    cursor: grabbing !important;
  }

  .board.grab :global(iframe),
  .board.grab :global(embed),
  .board.grab :global(object) {
    pointer-events: none;
  }

  .board {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background-color: var(--bg);
    background-image: radial-gradient(var(--dot) 1px, transparent 1px);
    cursor: default;
    touch-action: none;
    user-select: none;
  }

  .board.pen,
  .board.note,
  .board.arrow {
    cursor: crosshair;
  }

  .board.eraser {
    cursor: cell;
  }

  .board.placing {
    cursor: crosshair;
  }

  .board.dropover {
    box-shadow: inset 0 0 0 2px var(--acc);
  }

  .dropnote {
    position: absolute;
    left: 50%;
    top: 14px;
    transform: translateX(-50%);
    padding: 5px 10px;
    font-size: 10px;
    color: var(--acct);
    background: var(--accbg);
    border: 1px solid var(--accln);
    pointer-events: none;
  }

  /* A frame: an area under the cards it holds, its title above it. */
  .node.frame {
    background: color-mix(in srgb, var(--sel) 45%, transparent);
    border: 1.5px dashed var(--lines);
  }

  /* The frame's handle: picking and moving a frame is done by its title. */
  .ftitle {
    position: absolute;
    left: 0;
    top: -28px;
    min-width: 80px;
    max-width: 100%;
    padding: 4px 8px 4px 2px;
    font-size: 13px;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: move;
  }

  .ftitle:hover {
    color: var(--hi);
  }

  .ftitle .empty {
    color: var(--lab);
    font-style: italic;
  }

  .fedit {
    background: var(--inp);
    border: 1px solid var(--acc);
    color: var(--txt);
    font-size: 13px;
    padding: 1px 6px;
    outline: none;
    user-select: text;
  }

  .lassoline {
    fill: color-mix(in srgb, var(--acc) 10%, transparent);
    stroke: var(--acc);
    stroke-width: 1;
    stroke-dasharray: 4 3;
  }

  .marquee {
    position: absolute;
    border: 1px solid var(--acc);
    background: color-mix(in srgb, var(--acc) 10%, transparent);
    pointer-events: none;
  }

  .framing {
    position: absolute;
    border: 1.5px dashed var(--acc);
    background: var(--accbg);
    pointer-events: none;
  }

  /* Reference cards. */
  .node.file,
  .node.link,
  .node.question {
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: var(--card);
    border: 1px solid var(--lines);
    overflow: hidden;
  }

  .node.file {
    padding: 0;
  }

  .pic {
    display: block;
    width: 100%;
    flex: 1;
    min-height: 0;
    object-fit: contain;
    background: var(--bg);
    pointer-events: none;
  }

  .fhead,
  .lhead {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    min-width: 0;
  }

  .node.file .fhead {
    border-bottom: 1px solid var(--line);
  }

  .ext {
    font-size: 9px;
    letter-spacing: 0.1em;
    color: var(--acct);
    border: 1px solid var(--accln);
    padding: 1px 5px;
    flex-shrink: 0;
  }

  .fname,
  .ltitle {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    color: var(--hi);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .fopen {
    width: 22px;
    height: 22px;
    padding: 0;
    flex-shrink: 0;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--dim);
    font-size: 12px;
  }

  .fopen:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .pdf {
    flex: 1;
    min-height: 0;
    width: 100%;
    border: 0;
    background: #fff;
    pointer-events: none;
  }

  .pdf.live {
    pointer-events: auto;
  }

  .caption {
    padding: 6px 10px 8px;
    font-size: 12px;
    color: var(--dim);
    line-height: 1.45;
    white-space: pre-wrap;
    flex-shrink: 0;
  }

  textarea.caption {
    min-height: 48px;
  }

  .node.link {
    padding: 0 0 8px;
    border-left: 3px solid var(--acc);
  }

  .node.link .text,
  .node.link .edit {
    padding: 0 10px;
    font-size: 12.5px;
  }

  .lurl {
    padding: 0 10px;
    font-size: 10.5px;
    color: var(--lab);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .node.question {
    padding: 10px 12px;
    border-top: 2px solid var(--warn);
  }

  .qlab {
    font-size: 9px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--warn);
  }

  .answer {
    min-height: 44px;
    padding: 6px 8px;
    font-size: 13px;
    line-height: 1.45;
    color: var(--txt);
    background: var(--bg);
    border: 1px dashed var(--lines);
    white-space: pre-wrap;
    word-break: break-word;
    cursor: text;
    display: flex;
  }

  .answer .empty {
    color: var(--lab);
  }

  .answer .edit {
    min-height: 40px;
  }

  .linkin {
    position: absolute;
    width: 320px;
    height: 36px;
    padding: 0 10px;
    background: var(--card);
    border: 1px solid var(--acc);
    color: var(--txt);
    font-family: var(--mono);
    font-size: 12px;
    outline: none;
    user-select: text;
  }

  .world {
    position: absolute;
    left: 0;
    top: 0;
    transform-origin: 0 0;
  }

  .edges,
  .live {
    position: absolute;
    overflow: visible;
  }

  .edge .line {
    stroke: var(--lab);
    stroke-width: 1.6;
    fill: none;
    pointer-events: none;
  }

  .edge .hit {
    stroke: transparent;
    stroke-width: 12;
    cursor: pointer;
  }

  .edge.on .line {
    stroke: var(--acc);
  }

  .edge.pending .line {
    stroke: var(--acc);
    stroke-dasharray: 5 4;
  }

  .node {
    position: absolute;
    box-sizing: border-box;
    cursor: default;
  }

  .node.note {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    background: var(--card);
    border: 1px solid var(--lines);
    border-top: 2px solid var(--tag, var(--lines));
  }

  .node.sketch {
    border: 1px dashed transparent;
  }

  .node.on {
    outline: 1px solid var(--acc);
    outline-offset: 2px;
  }

  .node.from {
    outline: 2px solid var(--acc);
  }

  /* The agent's, until kept or reverted. */
  .node.pending.note {
    border-style: dashed;
    border-color: var(--acc);
    background: var(--accbg);
  }

  .node.pending.sketch {
    border-color: var(--acc);
  }

  .tag {
    font-size: 9px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--tag);
  }

  .text {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    font-size: 14px;
    line-height: 1.5;
    color: var(--txt);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .text.empty {
    color: var(--lab);
  }

  .edit {
    flex: 1;
    min-height: 0;
    width: 100%;
    resize: none;
    background: transparent;
    border: 0;
    outline: none;
    color: var(--txt);
    font-family: var(--sans);
    font-size: 14px;
    line-height: 1.5;
    padding: 0;
    user-select: text;
  }

  .grip {
    position: absolute;
    right: -5px;
    bottom: -5px;
    width: 10px;
    height: 10px;
    background: var(--acc);
    cursor: nwse-resize;
  }

  .ink {
    display: block;
    overflow: visible;
  }

  .ink .gone {
    opacity: 0.15;
  }

  .review {
    position: absolute;
    top: -26px;
    right: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 3px 0 7px;
    background: var(--card);
    border: 1px solid var(--accln);
  }

  .sug {
    font-size: 9px;
    letter-spacing: 0.12em;
    color: var(--acct);
    margin-right: 2px;
  }

  .review button {
    width: 18px;
    height: 18px;
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 0;
    color: var(--dim);
    font-size: 12px;
  }

  .review button:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .tools {
    position: absolute;
    left: 14px;
    top: 14px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 4px;
    background: var(--card);
    border: 1px solid var(--lines);
    cursor: default;
  }

  .tool {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    background: transparent;
    border: 0;
    color: var(--dim);
  }

  .tool:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .tool.on {
    color: var(--hi);
    background: var(--sel);
    box-shadow: inset 2px 0 0 var(--acc);
  }

  .sep {
    width: 20px;
    height: 1px;
    margin: 4px 0;
    background: var(--line);
  }

  .swatch {
    width: 18px;
    height: 18px;
    margin: 3px;
    padding: 0;
    border-radius: 50%;
    background: var(--c);
    border: 2px solid transparent;
  }

  .swatch.on {
    border-color: var(--hi);
  }

  .tagbar {
    position: absolute;
    left: 50%;
    top: 14px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px;
    background: var(--card);
    border: 1px solid var(--lines);
    cursor: default;
  }

  .lab {
    font-size: 9px;
    letter-spacing: 0.14em;
    color: var(--lab);
    margin-right: 4px;
  }

  .tagbtn {
    height: 24px;
    padding: 0 9px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--dim);
    font-size: 11px;
  }

  .tagbtn:hover {
    color: var(--hi);
  }

  .tagbtn.on {
    border-color: var(--tag);
    color: var(--tag);
  }

  .zoom {
    position: absolute;
    left: 14px;
    bottom: 14px;
    display: flex;
    background: var(--card);
    border: 1px solid var(--lines);
    cursor: default;
  }

  .zoom button {
    height: 28px;
    min-width: 28px;
    padding: 0 6px;
    background: transparent;
    border: 0;
    color: var(--dim);
    font-size: 13px;
  }

  .zoom button:hover {
    color: var(--hi);
    background: var(--sel);
  }

  .pct {
    font-size: 10px !important;
    min-width: 48px !important;
  }

  .hint {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    pointer-events: none;
  }

  .hint p {
    margin: 0;
    color: var(--lab);
  }

  .hint .serif {
    font-size: 22px;
    color: var(--dim);
  }

  .hint .mono {
    font-size: 11px;
    letter-spacing: 0.06em;
  }

  .mode {
    position: absolute;
    left: 50%;
    bottom: 14px;
    transform: translateX(-50%);
    padding: 5px 10px;
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--dim);
    background: var(--card);
    border: 1px solid var(--lines);
    pointer-events: none;
  }
</style>
