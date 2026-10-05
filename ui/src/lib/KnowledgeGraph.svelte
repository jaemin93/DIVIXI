<script module lang="ts">
  import { DEFAULT_PREFS, type CardPrefs } from "./detailCard";
  /**
   * Where each entity was last laid out, and where the camera was, kept for as
   * long as the app runs. The graph tab is mounted afresh every time it is
   * opened; without this it started from scratch and shook into place again.
   */
  const remembered = new Map<string, { x: number; y: number }>();
  /** Keyed by id and name together: another instance's library reuses the same ids for other entities. */
  const placeKey = (n: { id: number; name: string }) => `${n.id}:${n.name}`;
  let rememberedView: { x: number; y: number; k: number } | null = null;
  /** The library that camera was on (null: all): another library's plate is framed afresh. */
  let rememberedViewLibrary: number | null = null;
  /** The entity card's folds, kept across picks and visits to the tab. */
  let rememberedCard: CardPrefs = DEFAULT_PREFS;
  /**
   * The picked node and the centre (by placeKey, so another instance's entity
   * of the same id is not taken for them), the centre's depth, and "related
   * only": the graph's selection, kept across tabs.
   */
  let rememberedFocus: { picked: string | null; centre: string | null; depth: number; relatedOnly: boolean } = { picked: null, centre: null, depth: 1, relatedOnly: true };

  /** A marker's shape; the template's legend draws them too, so the type lives here. */
  type Glyph = "square" | "diamond" | "circle" | "block" | "dot" | "triangle";
</script>

<script lang="ts">
  import { kb, type KItem, type KNode } from "./knowledge.svelte";
  import { tick as nextTick, untrack } from "svelte";
  import { t } from "./i18n.svelte";
  import { placeNew, queryReach, settle, settleBudget, tick, visibleIds, STILL } from "./graphLayout";
  import { cardMaxHeight, cardRect, cardShape, listMinLeft, railVisible, toggled } from "./detailCard";
  import { contextChips } from "./graphContext";
  import { kindCounts, nodesOfKind } from "./kindList";
  import { store } from "./store.svelte";

  /**
   * The entity graph, drawn as a plate: a small force layout on a canvas
   * over paper and a faint grid, hairline edges, small markers sized by how
   * often an entity is mentioned. Type is told by the marker's shape, not a
   * colour; the one red goes to the hub (the best-connected entity) and to
   * what is pointed at or picked. Drag the empty space to pan, the wheel to
   * zoom, a node to move it; a click shows the node and the items that
   * mention it.
   */
  const KIND_GLYPHS: Record<string, Glyph> = {
    service: "square",
    technology: "diamond",
    concept: "circle",
    org: "block",
    person: "dot",
    api: "triangle",
  };

  type Body = { id: number; node: KNode; x: number; y: number; vx: number; vy: number; r: number; pinned: boolean };

  let canvas = $state<HTMLCanvasElement>();
  let picked = $state<KNode | null>(null);
  let pickedItems = $state<KItem[]>([]);
  /** Whether the entity card is folded to its header, and whether its sources are listed. */
  let card = $state<CardPrefs>(rememberedCard);
  /** The open card's height, for labels to keep clear of it. */
  let cardH = $state(0);
  /** The plate's height, and the query and centre boxes' at its bottom left, for the card to stop short of. */
  let stageH = $state(0);
  let boxesH = $state(0);
  /** The boxes' distance from the plate's bottom (their CSS `bottom`). */
  const BOXES_BOTTOM = 34;
  const shape = $derived(cardShape(!!picked, card));
  /** The plate's width, which the agent panel takes from. */
  let stageW = $state(0);
  const railOn = $derived(railVisible(shape, stageW));

  function flip(key: keyof CardPrefs) {
    card = toggled(card, key);
    rememberedCard = card;
    kick();
  }

  let bodies: Body[] = [];
  let links: { a: Body; b: Body }[] = [];
  /** What is simulated and drawn: every body, or only a focus's. */
  let shown: Body[] = [];
  let shownLinks: { a: Body; b: Body }[] = [];
  let view = rememberedView ? { ...rememberedView } : { x: 0, y: 0, k: 1 };
  let frame = 0;
  /** The live layout's temperature. Zero once settled: a still plate stays still. */
  let heat = 0;

  /**
   * Focus: a part of the graph instead of all of it. A node chosen as the
   * centre shows it and its neighbours `depth` relations out; otherwise a
   * shown query shows only the entities it reaches, unless the person asked
   * for everything. `visible` is null when the whole graph is shown.
   */
  let centre = $state<number | null>(kb.graph.nodes.find((n) => placeKey(n) === rememberedFocus.centre)?.id ?? null);
  let depth = $state(rememberedFocus.depth);
  let relatedOnly = $state(rememberedFocus.relatedOnly);
  let visible = $state<Set<number> | null>(null);
  /** The whole graph's positions, put aside while a focus moves its part around. */
  let fullPos: Map<number, { x: number; y: number }> | null = null;

  /** The best-connected entity, drawn as the plate's centre mark. */
  let hub: Body | null = null;
  let hubLinks = 0;
  let hovered: Body | null = null;
  /** Where the pointer is, in world units, for the coordinate readout. */
  let pointer: { x: number; y: number } | null = null;

  /**
   * The overview rail's current section: "*" for the whole plate, an
   * entity kind for that kind's nodes, null once nothing has been chosen.
   * A kind keeps its nodes in full ink and fades the rest.
   */
  let section = $state<string | null>(null);
  /**
   * The entities the shown search reaches: the core's answer, widened to
   * nodes whose names hold the query's words (see queryReach). Empty when
   * there is no query or it reaches nothing.
   */
  const reach = $derived(
    kb.shownQuery ? queryReach(kb.queryEntities, kb.graph.nodes, kb.graph.edges, kb.shownQuery) : { seeds: new Set<number>(), related: new Set<number>() },
  );
  const queried = $derived(reach.related);
  const seeded = $derived(reach.seeds);
  /** Documents the shown search found passages in. */
  const querySources = $derived(new Set(kb.items.map((i) => i.source_id)).size);

  $effect(() => {
    // A new query lights other nodes: draw again.
    void queried;
    kick();
  });

  /** A camera move in progress, eased from one view to another. */
  let glide: { from: typeof view; to: typeof view; t0: number } | null = null;
  const GLIDE_MS = 420;

  function build() {
    const g = kb.graph;
    // A rebuild while focused starts from the whole layout, not the focus's.
    if (fullPos) restore(fullPos);
    fullPos = null;
    visible = null;
    const old = new Map(bodies.map((b) => [b.node.id, b]));
    const placed = new Set<number>();
    bodies = g.nodes.map((node) => {
      const prev = old.get(node.id) ?? remembered.get(placeKey(node));
      if (prev) placed.add(node.id);
      return {
        id: node.id,
        node,
        x: prev?.x ?? 0,
        y: prev?.y ?? 0,
        vx: 0,
        vy: 0,
        r: 2.5 + Math.min(4, Math.sqrt(node.mentions) * 1.1),
        pinned: old.get(node.id)?.pinned ?? false,
      };
    });
    const byId = new Map(bodies.map((b) => [b.node.id, b]));
    links = g.edges.flatMap((e) => {
      const a = byId.get(e.source);
      const b = byId.get(e.target);
      return a && b ? [{ a, b }] : [];
    });
    // Lay out what has no place yet, off screen, before anything is drawn:
    // the first frame shows a settled plate, not an explosion. Nodes that
    // already had a place keep it, so a reload or a return to the tab does
    // not move them.
    const fresh = bodies.length - placed.size;
    placeNew(bodies, g.edges, placed);
    if (fresh) settle(bodies, links, fresh === bodies.length ? 1 : 0.35, settleBudget(bodies.length));
    remember();
    if (hovered && !bodies.includes(hovered)) hovered = null;
    // A reload may take the chosen kind away.
    if (section && section !== "*" && !g.nodes.some((n) => n.kind === section)) section = null;
    // The node picked before the tab was left is picked again.
    if (!picked && rememberedFocus.picked !== null) {
      const again = g.nodes.find((n) => placeKey(n) === rememberedFocus.picked);
      if (again) void pick(again);
    }
    // A centre the reload (another library) took away is let go.
    if (centre !== null && !g.nodes.some((n) => n.id === centre)) centre = null;
    // The camera is framed the first time this tab draws anything (unless it
    // remembers where it was, on this library); after, a reload leaves it
    // where the person put it, and another library's graph is framed.
    const first = !built;
    const otherLibrary = first ? rememberedViewLibrary !== kb.library : builtLibrary !== kb.library;
    built = true;
    builtLibrary = kb.library;
    builtGraph = g;
    applyFocus(first && (!rememberedView || otherLibrary) ? "jump" : otherLibrary ? "ease" : "keep");
  }

  /** Whether this mount has drawn the graph yet. */
  let built = false;
  /** The library and the graph it last drew: a reload is told from a new question by them. */
  let builtLibrary: number | null = null;
  let builtGraph: unknown = null;

  /** The best-connected node of what is shown: most links, then most mentions; none without a web. */
  function findHub() {
    const degree = new Map<Body, number>();
    for (const { a, b } of shownLinks) {
      degree.set(a, (degree.get(a) ?? 0) + 1);
      degree.set(b, (degree.get(b) ?? 0) + 1);
    }
    hub = null;
    hubLinks = 0;
    for (const b of shown) {
      const d = degree.get(b) ?? 0;
      if (d > hubLinks || (d === hubLinks && hub && b.node.mentions > hub.node.mentions)) {
        hub = b;
        hubLinks = d;
      }
    }
    if (hubLinks < 3) hub = null;
  }

  function snapshot(): Map<number, { x: number; y: number }> {
    return new Map(bodies.map((b) => [b.id, { x: b.x, y: b.y }]));
  }

  function restore(pos: Map<number, { x: number; y: number }>) {
    for (const b of bodies) {
      const p = pos.get(b.id);
      if (p) {
        b.x = p.x;
        b.y = p.y;
      }
      b.vx = 0;
      b.vy = 0;
    }
  }

  /** Keep the whole layout for the next time the tab opens. */
  function remember() {
    const pos = fullPos ?? snapshot();
    for (const b of bodies) {
      const p = pos.get(b.id);
      if (p) remembered.set(placeKey(b.node), p);
    }
  }

  /**
   * Show what the focus asks for. A focus lays its part out on its own,
   * gathered round the middle and settled off screen, then frames it; leaving
   * the focus puts the whole layout back exactly as it was.
   */
  function applyFocus(camera: "keep" | "ease" | "jump") {
    const next = visibleIds({ nodes: kb.graph.nodes, edges: kb.graph.edges, related: [...queried], relatedOnly, centre, depth });
    if (fullPos) restore(fullPos);
    fullPos = next ? (fullPos ?? snapshot()) : null;
    visible = next;
    shown = next ? bodies.filter((b) => next.has(b.id)) : bodies;
    const inShown = new Set(shown);
    shownLinks = next ? links.filter((l) => inShown.has(l.a) && inShown.has(l.b)) : links;
    if (next && shown.length) {
      const cx = shown.reduce((sum, b) => sum + b.x, 0) / shown.length;
      const cy = shown.reduce((sum, b) => sum + b.y, 0) / shown.length;
      for (const b of shown) {
        b.x -= cx;
        b.y -= cy;
      }
      settle(shown, shownLinks, 0.6, settleBudget(shown.length));
    }
    findHub();
    heat = 0;
    if (camera !== "keep") frameAll(camera === "ease");
    kick();
  }

  // The card's height lands after layout; labels it covers are placed again then.
  $effect(() => {
    void cardH;
    untrack(() => kick());
  });

  /** A focus's inputs changed: the query's entities, "show all", the centre or its depth. */
  $effect(() => {
    void queried;
    void relatedOnly;
    void centre;
    void depth;
    // Only the inputs above: what applyFocus reads besides is not a reason to run it.
    // A reloaded graph changes them too; build() frames that one, and runs after.
    untrack(() => {
      if (bodies.length && kb.graph === builtGraph) applyFocus("ease");
    });
  });

  /** A new search asks a new question: show what it reaches, not an old centre. */
  let lastQuery = kb.shownQuery;
  $effect(() => {
    if (kb.shownQuery !== lastQuery) {
      lastQuery = kb.shownQuery;
      relatedOnly = true;
      centre = null;
    }
  });

  /** Below this the layout has settled and the loop rests. */
  const SETTLED = 0.021;
  /** Theme colours, read when an animation starts rather than every frame. */
  let ink = {
    paper: "#f3f1ed",
    grid: "#ebe8e2",
    grid2: "#e1ddd6",
    edge: "#c2bdb4",
    edgeAcc: "#dbb2ad",
    node: "#35332f",
    acc: "#8f1d16",
    label: "#35332f",
    meta: "#6b675f",
    mono: "monospace",
  };

  /** Start drawing again (the layout or the view changed). */
  function kick() {
    if (frame || !canvas) return;
    readInk();
    frame = requestAnimationFrame(loop);
  }

  function readInk() {
    if (!canvas) return;
    const styles = getComputedStyle(canvas);
    const v = (name: string, fallback: string) => styles.getPropertyValue(name).trim() || fallback;
    ink = {
      paper: v("--kg-paper", ink.paper),
      grid: v("--kg-grid", ink.grid),
      grid2: v("--kg-grid2", ink.grid2),
      edge: v("--kg-edge", ink.edge),
      edgeAcc: v("--kg-edge-acc", ink.edgeAcc),
      node: v("--kg-node", ink.node),
      acc: v("--kg-acc", ink.acc),
      label: v("--body", ink.label),
      meta: v("--lab", ink.meta),
      mono: v("--mono", ink.mono),
    };
  }

  /** How fast the live layout cools after a drag; it is still within a second or so. */
  const COOLING = 0.95;

  function glyphOf(kind: string): Glyph {
    return KIND_GLYPHS[kind] ?? "square";
  }

  /** A marker's outline at (x, y), half-size r, as the current path. */
  function glyphPath(ctx: CanvasRenderingContext2D, g: Glyph, x: number, y: number, r: number) {
    ctx.beginPath();
    if (g === "circle" || g === "dot") ctx.arc(x, y, r, 0, Math.PI * 2);
    else if (g === "diamond") {
      const d = r * 1.3;
      ctx.moveTo(x, y - d);
      ctx.lineTo(x + d, y);
      ctx.lineTo(x, y + d);
      ctx.lineTo(x - d, y);
      ctx.closePath();
    } else if (g === "triangle") {
      const d = r * 1.25;
      ctx.moveTo(x, y - d);
      ctx.lineTo(x + d, y + d * 0.8);
      ctx.lineTo(x - d, y + d * 0.8);
      ctx.closePath();
    } else ctx.rect(x - r, y - r, r * 2, r * 2);
  }

  /** Letter-spacing for the house micro-label; a canvas that does not know it just ignores it. */
  function spaced(ctx: CanvasRenderingContext2D, px: number) {
    (ctx as CanvasRenderingContext2D & { letterSpacing?: string }).letterSpacing = `${px}px`;
  }

  /** A graph this small labels every node it has room for, not just the often mentioned. */
  const SMALL_GRAPH = 40;
  /** Characters a label shows before it is cut short. */
  const LABEL_MAX = 28;

  /** The hub ring's radius on screen at zoom k. */
  const ringOf = (k: number) => 14 * Math.min(1.4, Math.max(0.8, k));

  function draw() {
    const c = canvas;
    if (!c) return;
    const ctx = c.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth;
    const h = c.clientHeight;
    if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) {
      c.width = Math.round(w * dpr);
      c.height = Math.round(h * dpr);
    }
    const k = view.k;
    const ox = w / 2 + view.x;
    const oy = h / 2 + view.y;
    // World to screen. Everything is drawn in screen pixels so hairlines and text stay crisp at any zoom.
    const sx = (x: number) => ox + x * k;
    const sy = (y: number) => oy + y * k;
    const snap = (v: number) => Math.round(v) + 0.5;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = ink.paper;
    ctx.fillRect(0, 0, w, h);

    // The grid moves with the plate: a fine step, every fifth line a shade darker.
    let step = 40;
    while (step * k < 18) step *= 5;
    const gx0 = Math.floor(-ox / k / step) * step;
    const gy0 = Math.floor(-oy / k / step) * step;
    ctx.lineWidth = 1;
    for (const major of [false, true]) {
      ctx.strokeStyle = major ? ink.grid2 : ink.grid;
      ctx.beginPath();
      for (let x = gx0; sx(x) <= w; x += step) {
        if ((Math.round(x / step) % 5 === 0) !== major) continue;
        ctx.moveTo(snap(sx(x)), 0);
        ctx.lineTo(snap(sx(x)), h);
      }
      for (let y = gy0; sy(y) <= h; y += step) {
        if ((Math.round(y / step) % 5 === 0) !== major) continue;
        ctx.moveTo(0, snap(sy(y)));
        ctx.lineTo(w, snap(sy(y)));
      }
      ctx.stroke();
    }

    const pick = picked ? shown.find((b) => b.node.id === picked!.id) : undefined;
    const lit = (b: Body) => b === pick || b === hovered;
    // Two things fade a node: the overview rail's kind, and the shown query.
    // The query is the question asked, so it outranks the rail: a node the
    // query reaches stays half-inked outside the chosen kind, and one it
    // does not reach fades furthest whatever kind it is.
    const focus = section && section !== "*" ? section : null;
    const asked = queried.size > 0;
    const ink1 = (b: Body): number => {
      if (lit(b)) return 1;
      const inKind = !focus || b.node.kind === focus;
      if (asked && !queried.has(b.node.id)) return 0.16;
      return inKind ? 1 : asked ? 0.45 : 0.25;
    };
    const faded = (b: Body) => ink1(b) < 1;
    const size = (b: Body) => Math.max(2.5, b.r * Math.min(1.6, Math.max(0.7, k)));

    // The hub's orbit: a dotted ring at the mean distance of its neighbours.
    if (hub) {
      let sum = 0;
      let n = 0;
      for (const { a, b } of shownLinks) {
        const o = a === hub ? b : b === hub ? a : null;
        if (!o) continue;
        sum += Math.hypot(o.x - hub.x, o.y - hub.y);
        n++;
      }
      if (n) {
        ctx.strokeStyle = ink.edge;
        ctx.setLineDash([1, 5]);
        ctx.beginPath();
        ctx.arc(sx(hub.x), sy(hub.y), (sum / n) * k, 0, Math.PI * 2);
        ctx.stroke();
        ctx.setLineDash([]);
      }
    }

    // Hairline edges in three inks: plain, touching the hub, touching what is lit.
    const plain: typeof links = [];
    const warm: typeof links = [];
    const hot: typeof links = [];
    const dim: typeof links = [];
    for (const l of shownLinks) {
      if (lit(l.a) || lit(l.b)) hot.push(l);
      else if (faded(l.a) || faded(l.b)) dim.push(l);
      else if (asked || l.a === hub || l.b === hub) warm.push(l);
      else plain.push(l);
    }
    for (const [set, color, width, alpha] of [
      [dim, ink.edge, 0.75, 0.35],
      [plain, ink.edge, 0.75, 1],
      [warm, ink.edgeAcc, 0.9, 1],
      [hot, ink.acc, 1, 1],
    ] as const) {
      if (!set.length) continue;
      ctx.globalAlpha = alpha;
      ctx.strokeStyle = color;
      ctx.lineWidth = width;
      ctx.beginPath();
      for (const { a, b } of set) {
        ctx.moveTo(sx(a.x), sy(a.y));
        ctx.lineTo(sx(b.x), sy(b.y));
      }
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    // Markers: hollow on paper, solid for the filled kinds, red when lit.
    ctx.lineWidth = 1;
    for (const b of shown) {
      if (b === hub) continue;
      const g = glyphOf(b.node.kind);
      const r = size(b);
      const x = snap(sx(b.x));
      const y = snap(sy(b.y));
      const on = lit(b);
      const solid = g === "block" || g === "dot";
      // What the query names is drawn in the accent, like what is pointed at.
      const named = on || seeded.has(b.node.id);
      ctx.globalAlpha = ink1(b);
      glyphPath(ctx, g, x, y, r);
      ctx.fillStyle = b === pick ? ink.acc : solid ? (named ? ink.acc : ink.node) : ink.paper;
      ctx.fill();
      ctx.strokeStyle = named ? ink.acc : ink.node;
      ctx.stroke();
      if (b === pick) {
        ctx.strokeStyle = ink.acc;
        ctx.strokeRect(x - r - 4, y - r - 4, (r + 4) * 2, (r + 4) * 2);
      }
    }
    ctx.globalAlpha = 1;

    // The hub: a red ring with a dot at its centre.
    if (hub) {
      const x = sx(hub.x);
      const y = sy(hub.y);
      ctx.globalAlpha = Math.max(0.3, ink1(hub));
      ctx.beginPath();
      ctx.arc(x, y, ringOf(k), 0, Math.PI * 2);
      ctx.fillStyle = ink.paper;
      ctx.fill();
      ctx.strokeStyle = ink.acc;
      ctx.lineWidth = lit(hub) ? 1.6 : 1;
      ctx.stroke();
      ctx.fillStyle = ink.acc;
      ctx.beginPath();
      ctx.arc(x, y, 2.6, 0, Math.PI * 2);
      ctx.fill();
      ctx.lineWidth = 1;
      ctx.globalAlpha = 1;
    }

    // Labels: spaced mono capitals; under the ones that matter, a line of grey meta.
    // Placed in order of importance (what is lit, the hub, the most mentioned), and a
    // label that would land on one already placed is left out rather than overprinted.
    const order = shown
      .filter((b) => (!faded(b) || seeded.has(b.node.id)) && (b === hub || lit(b) || seeded.has(b.node.id) || b.node.mentions >= 2 || shown.length <= SMALL_GRAPH || k > 1.4))
      .sort(
        (p, q) =>
          Number(lit(q)) - Number(lit(p)) ||
          Number(seeded.has(q.node.id)) - Number(seeded.has(p.node.id)) ||
          Number(q === hub) - Number(p === hub) ||
          q.node.mentions - p.node.mentions,
      );
    // Markers in full ink are already on the plate: no label may cover one. Faded ones may be written over.
    const taken: [number, number, number, number][] = shown.filter((b) => !faded(b)).map((b) => {
      const r = b === hub ? ringOf(k) : size(b) * 1.3;
      return [sx(b.x) - r, sy(b.y) - r, sx(b.x) + r, sy(b.y) + r];
    });
    // Nor may one run under the overview rail on the right edge.
    const railH = sections.length * 24;
    if (railOn) taken.push([w - 150, h / 2 - railH / 2 - 8, w, h / 2 + railH / 2 + 8]);
    // Nor under the entity card, open or folded, top left.
    const covered = cardRect(shape, w, cardH);
    if (covered) taken.push(covered);
    const free = (x0: number, y0: number, x1: number, y1: number) =>
      !taken.some(([a0, b0, a1, b1]) => x0 < a1 && x1 > a0 && y0 < b1 && y1 > b0);
    for (const b of order) {
      const on = lit(b);
      const isHub = b === hub;
      const x = sx(b.x);
      const y = sy(b.y);
      if (x < -240 || x > w + 40 || y < -30 || y > h + 30) continue;
      // Long names (hosts, URLs) are cut short until pointed at or picked.
      const full = b.node.name.toUpperCase();
      const name = on || full.length <= LABEL_MAX ? full : `${full.slice(0, LABEL_MAX - 1)}…`;
      const meta = isHub ? t("kb.graphHub", { n: hubLinks }).toUpperCase() : t("kb.graphMeta", { kind: b.node.kind, n: b.node.mentions });
      const showMeta = isHub || on || k > 1.2;
      ctx.font = `${isHub ? 500 : 400} 10px ${ink.mono}`;
      spaced(ctx, 1.4);
      const nw = ctx.measureText(name).width;
      ctx.font = `9px ${ink.mono}`;
      spaced(ctx, 0.6);
      const mw = showMeta ? ctx.measureText(meta).width : 0;
      const r = isHub ? ringOf(k) : size(b);
      // Hub labels hang centred under the ring; the rest sit to the right of the marker.
      // A picked node's label clears the crosshair drawn round it.
      const lx = isHub ? x - Math.max(nw, mw) / 2 : x + r + (b === pick ? 22 : 6);
      const ly = isHub ? y + r + 6 : y - 6;
      const box: [number, number, number, number] = [lx - 2, ly, lx + Math.max(nw, mw) + 2, ly + (showMeta ? 24 : 12)];
      const forced = on || seeded.has(b.node.id);
      if (!forced && !free(...box)) continue;
      taken.push(box);
      if (forced) {
        // Lit and named labels sit on a patch of paper so they read over whatever is under them.
        ctx.fillStyle = ink.paper;
        ctx.fillRect(box[0], box[1], box[2] - box[0], box[3] - box[1]);
      }
      ctx.font = `${isHub ? 500 : 400} 10px ${ink.mono}`;
      spaced(ctx, 1.4);
      ctx.fillStyle = on || seeded.has(b.node.id) ? ink.acc : ink.label;
      ctx.fillText(name, lx + (isHub ? (Math.max(nw, mw) - nw) / 2 : 0), ly + 9.5);
      if (showMeta) {
        ctx.font = `9px ${ink.mono}`;
        spaced(ctx, 0.6);
        ctx.fillStyle = ink.meta;
        ctx.fillText(meta, lx + (isHub ? (Math.max(nw, mw) - mw) / 2 : 0), ly + 21);
      }
      // A bracketed count over what is lit, like a plate's reference mark.
      if (on && !isHub) {
        ctx.fillStyle = ink.acc;
        ctx.textAlign = "center";
        ctx.fillText(`[${b.node.mentions}]`, x, y - r - 8);
        ctx.textAlign = "left";
      }
    }

    // A crosshair and its coordinates beside what is picked.
    if (pick) {
      const x = snap(sx(pick.x));
      const y = snap(sy(pick.y));
      const gap = (pick === hub ? ringOf(k) : size(pick) + 4) + 6;
      ctx.strokeStyle = ink.meta;
      ctx.beginPath();
      for (const [dx, dy] of [
        [1, 0],
        [-1, 0],
        [0, 1],
        [0, -1],
      ]) {
        ctx.moveTo(x + dx * gap, y + dy * gap);
        ctx.lineTo(x + dx * (gap + 9), y + dy * (gap + 9));
      }
      ctx.stroke();
      ctx.font = `9px ${ink.mono}`;
      spaced(ctx, 1);
      ctx.fillStyle = ink.meta;
      ctx.textAlign = "right";
      const coords = `X ${Math.round(pick.x)}  Y ${Math.round(pick.y)}`;
      const cw = ctx.measureText(coords).width;
      ctx.fillStyle = ink.paper;
      ctx.fillRect(x - gap - 6 - cw, y - gap - 13, cw + 4, 12);
      ctx.fillStyle = ink.meta;
      ctx.fillText(coords, x - gap - 4, y - gap - 4);
      ctx.textAlign = "left";
    }

    // Along the foot of the plate: the pointer's coordinates and a scale bar of 100 units.
    ctx.font = `9px ${ink.mono}`;
    spaced(ctx, 1.3);
    ctx.fillStyle = ink.meta;
    ctx.strokeStyle = ink.meta;
    if (pointer) ctx.fillText(`X ${Math.round(pointer.x)}  Y ${Math.round(pointer.y)}`, 14, h - 14);
    const unit = Math.round(100 * k);
    const right = w - 14;
    const base = h - 18.5;
    ctx.beginPath();
    ctx.moveTo(right - unit, base);
    ctx.lineTo(right, base);
    for (let i = 0; i <= 4; i++) {
      const tx = snap(right - (unit * i) / 4);
      ctx.moveTo(tx, base);
      ctx.lineTo(tx, base - (i % 4 === 0 ? 6 : 3));
    }
    ctx.stroke();
    ctx.textAlign = "right";
    ctx.fillText(`${t("kb.graphScale").toUpperCase()} ×${k.toFixed(2)}`, right - unit - 10, h - 14);
    ctx.textAlign = "left";
    spaced(ctx, 0);
  }

  function loop() {
    if (heat > SETTLED) {
      const moved = tick(shown, shownLinks, heat);
      heat *= COOLING;
      // Stop as soon as nothing visibly moves, rather than idling warm and trembling.
      if (moved < STILL && !drag) heat = 0;
    }
    if (glide) {
      const p = Math.min(1, (performance.now() - glide.t0) / GLIDE_MS);
      const e = 1 - (1 - p) ** 3;
      const { from, to } = glide;
      view = { x: from.x + (to.x - from.x) * e, y: from.y + (to.y - from.y) * e, k: from.k + (to.k - from.k) * e };
      if (p === 1) glide = null;
    }
    draw();
    // Keep going while the layout moves, a node is held or the camera glides; otherwise rest until kicked.
    frame = heat > SETTLED || drag || glide ? requestAnimationFrame(loop) : 0;
    if (!frame) {
      rememberedView = { ...view };
      rememberedViewLibrary = kb.library;
      remember();
    }
  }

  /** A section is never framed further out than this, however its nodes lie. */
  const FRAME_MIN_K = 0.5;

  /** The range of `vs` without its outliers: 10th to 90th percentile once there are five or more. */
  function spread(vs: number[]): [number, number] {
    const sorted = [...vs].sort((a, b) => a - b);
    const n = sorted.length;
    if (n < 5) return [sorted[0], sorted[n - 1]];
    return [sorted[Math.floor((n - 1) * 0.1)], sorted[Math.ceil((n - 1) * 0.9)]];
  }

  /** Frame what is shown; `ease` glides there, otherwise the camera is simply put there. */
  function frameAll(ease: boolean) {
    frame_(shown, ease);
  }

  /** Move the camera to frame a section of the plate: everything, or one kind's nodes. */
  function goTo(next: string) {
    section = next;
    // The rail and the legend are one choice: an open list follows the rail.
    if (listKind && next !== listKind) listKind = next === "*" ? null : next;
    const set = next === "*" ? shown : shown.filter((b) => b.node.kind === next);
    if (!set.length || !canvas) {
      kick();
      return;
    }
    frame_(set, true);
  }

  function frame_(set: Body[], ease: boolean) {
    if (!set.length || !canvas || !canvas.clientWidth) return;
    // Frame the bulk of the set, not its strays: one node flung far out would
    // otherwise zoom the whole plate away. Past a handful of nodes the box runs
    // from the 10th to the 90th percentile on each axis.
    const [x0, x1] = spread(set.map((b) => b.x));
    const [y0, y1] = spread(set.map((b) => b.y));
    // Room for labels to the right, and for the rail, caption and scale bar round the edge.
    const w = canvas.clientWidth - 240;
    const h = canvas.clientHeight - 120;
    const k = Math.min(2.5, Math.max(FRAME_MIN_K, Math.min(w / Math.max(1, x1 - x0 + 120), h / Math.max(1, y1 - y0 + 40))));
    const cx = (x0 + x1) / 2 + 40;
    const cy = (y0 + y1) / 2;
    const to = { x: -cx * k, y: -cy * k, k };
    if (ease) glide = { from: { ...view }, to, t0: performance.now() };
    else {
      glide = null;
      view = to;
    }
    kick();
  }

  /** The rail: the whole plate first, then one stop per kind, most numerous first. */
  const sections = $derived.by(() => {
    const count = new Map<string, number>();
    const nodes = visible ? kb.graph.nodes.filter((n) => visible!.has(n.id)) : kb.graph.nodes;
    for (const n of nodes) count.set(n.kind, (count.get(n.kind) ?? 0) + 1);
    const byKind = [...count].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
    return [{ id: "*", label: t("kb.graphOverview"), n: nodes.length }, ...byKind.map(([id, n]) => ({ id, label: id, n }))];
  });

  $effect(() => {
    // Rebuild whenever the graph is reloaded, and for nothing else.
    void kb.graph;
    untrack(build);
  });

  $effect(() => {
    if (!canvas) return;
    kick();
    // A still plate draws once; when its box changes size (the window, the
    // panels, the page settling in) it has to draw again, or the browser
    // stretches the old picture to fit.
    const sized = new ResizeObserver(() => kick());
    sized.observe(canvas);
    return () => {
      sized.disconnect();
      cancelAnimationFrame(frame);
      frame = 0;
    };
  });

  $effect(() => {
    // The theme is a class on <html>: when it turns, take the new inks at once,
    // even mid-animation (kick alone only reads them when a loop starts).
    const seen = new MutationObserver(() => {
      readInk();
      kick();
    });
    seen.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => seen.disconnect();
  });

  $effect(() => {
    // Labels are set in the house mono: draw again once it has arrived.
    void document.fonts?.ready.then(() => kick());
  });

  function toWorld(e: PointerEvent | WheelEvent): { x: number; y: number } {
    const rect = canvas!.getBoundingClientRect();
    return {
      x: (e.clientX - rect.left - rect.width / 2 - view.x) / view.k,
      y: (e.clientY - rect.top - rect.height / 2 - view.y) / view.k,
    };
  }

  function hit(p: { x: number; y: number }): Body | undefined {
    for (let i = shown.length - 1; i >= 0; i--) {
      const b = shown[i];
      const r = Math.max(b === hub ? 16 : b.r + 3, 6) / view.k;
      if ((b.x - p.x) ** 2 + (b.y - p.y) ** 2 <= r * r) return b;
    }
    return undefined;
  }

  let drag: { body?: Body; sx: number; sy: number; vx: number; vy: number; moved: boolean } | null = null;

  function down(e: PointerEvent) {
    canvas!.setPointerCapture(e.pointerId);
    const body = hit(toWorld(e));
    drag = { body, sx: e.clientX, sy: e.clientY, vx: view.x, vy: view.y, moved: false };
    if (body) body.pinned = true;
    kick();
  }

  function move(e: PointerEvent) {
    pointer = toWorld(e);
    if (!drag) {
      const over = hit(pointer) ?? null;
      if (over !== hovered) {
        hovered = over;
        canvas!.style.cursor = over ? "pointer" : "";
      }
      kick();
      return;
    }
    if (Math.abs(e.clientX - drag.sx) + Math.abs(e.clientY - drag.sy) > 3) drag.moved = true;
    if (drag.body) {
      const p = toWorld(e);
      drag.body.x = p.x;
      drag.body.y = p.y;
      heat = Math.max(heat, 0.3);
    } else {
      glide = null;
      view.x = drag.vx + e.clientX - drag.sx;
      view.y = drag.vy + e.clientY - drag.sy;
    }
    kick();
  }

  function leave() {
    pointer = null;
    hovered = null;
    if (canvas) canvas.style.cursor = "";
    kick();
  }

  async function up() {
    if (!drag) return;
    const { body, moved } = drag;
    drag = null;
    kick();
    if (body && !moved) {
      body.pinned = false;
      await pick(body.node);
    } else if (!body && !moved) {
      picked = null;
      pickedItems = [];
    }
  }

  function wheel(e: WheelEvent) {
    e.preventDefault();
    glide = null;
    const before = toWorld(e);
    const k = Math.min(4, Math.max(0.25, view.k * Math.exp(-e.deltaY * 0.0015)));
    view.k = k;
    const after = toWorld(e);
    view.x += (after.x - before.x) * k;
    view.y += (after.y - before.y) * k;
    kick();
  }

  function recenter() {
    section = null;
    for (const b of bodies) b.pinned = false;
    frameAll(true);
  }

  function clearQuery() {
    kb.query = "";
    void kb.search("");
  }

  /** Pick a node: its card, and the passages that mention it. */
  async function pick(node: KNode) {
    const id = node.id;
    picked = node;
    pickedItems = [];
    const items = await kb.entityItems(id);
    // Another node may have been picked meanwhile.
    if (picked?.id === id) pickedItems = items;
  }

  $effect(() => {
    const centreNode = centre === null ? undefined : kb.graph.nodes.find((n) => n.id === centre);
    rememberedFocus = { picked: picked ? placeKey(picked) : null, centre: centreNode ? placeKey(centreNode) : null, depth, relatedOnly };
  });

  function focusOn(id: number) {
    centre = id;
  }

  /** The legend: every kind the graph has, most numerous first, with its count. */
  const kinds = $derived(kindCounts(kb.graph.nodes));

  // ----- a kind's list: the legend opens it -----

  /** The kind whose nodes are listed, or null. It is the rail's section too. */
  let listKind = $state<string | null>(null);
  let listFilter = $state("");
  const rows = $derived(listKind ? nodesOfKind(kb.graph.nodes, kb.graph.edges, listKind, listFilter) : []);

  /** The legend's buttons by kind, and the plate: the list hangs under its kind's button. */
  const legendEls: Record<string, HTMLButtonElement> = $state({});
  let stageEl = $state<HTMLDivElement>();
  /** The list's width: 280 px, or the plate less its margins when the plate is narrower. */
  const listW = $derived(Math.max(0, Math.min(280, stageW - 24)));
  /** The list's left edge in the plate: under its legend entry, kept inside the plate. */
  const listLeft = $derived.by(() => {
    const button = listKind ? legendEls[listKind] : undefined;
    if (!button || !stageEl) return 12;
    void stageW; // again when the plate's width (and so the legend's wrap) changes
    const x = button.getBoundingClientRect().left - stageEl.getBoundingClientRect().left;
    // Beside the entity card when one is shown; on a plate too narrow for both, over it.
    return Math.round(Math.max(12, Math.min(Math.max(x, listMinLeft(shape)), stageW - listW - 12)));
  });

  /** A legend entry: list its kind's nodes and frame them, or (again) close the list and show everything. */
  function toggleKind(kind: string) {
    if (listKind === kind) {
      listKind = null;
      goTo("*");
      return;
    }
    listKind = kind;
    listFilter = "";
    goTo(kind);
  }

  /** From the list: pick the node, bring it to the middle, and open the agent on it with the box ready. */
  async function askAbout(id: number) {
    const node = kb.graph.nodes.find((n) => n.id === id);
    if (!node) return;
    // A node outside the current focus is not drawn: show the whole graph so it can be.
    if (visible && !visible.has(id)) {
      centre = null;
      relatedOnly = false;
      await nextTick();
    }
    void pick(node);
    const b = shown.find((x) => x.id === id);
    if (b) {
      const k = Math.max(view.k, 1);
      glide = { from: { ...view }, to: { x: -b.x * k, y: -b.y * k, k }, t0: performance.now() };
      kick();
    }
    // Its chip goes in even if it was taken out before.
    if (store.graphExcluded.has(`entity:${id}`)) store.toggleGraphChip(`entity:${id}`);
    if (!store.graphChatOpen) await store.openGraphChat();
    await nextTick();
    document.querySelector<HTMLTextAreaElement>("aside.talk textarea")?.focus();
  }

  function closeList(e?: Event) {
    e?.stopPropagation();
    listKind = null;
  }
  const centreNode = $derived(centre === null ? null : (kb.graph.nodes.find((n) => n.id === centre) ?? null));

  // ----- the agent panel: what goes with a message -----

  const chips = $derived(
    contextChips({
      picked: picked ? { id: picked.id, name: picked.name } : null,
      pickedItems,
      centre: centreNode ? { id: centreNode.id, name: centreNode.name, depth } : null,
      focusIds: centre !== null && visible ? [...visible] : [],
      query: kb.shownQuery,
      queryIds: kb.shownQuery ? [...queried] : [],
    }),
  );
  $effect(() => {
    // The panel (a column of the app's layout) shows these; the composer sends their picks.
    store.graphChips = chips;
  });

  const sourceName = (id: string) => {
    const s = kb.sources.find((x) => x.id === id);
    return s ? kb.nameOf(s) : id;
  };
</script>

<!-- The kind list closes on Escape and on a click anywhere but the list and the legend. -->
<svelte:window
  onkeydown={(e) => {
    if (e.key === "Escape" && listKind) listKind = null;
  }}
  onpointerdown={(e) => {
    if (listKind && !(e.target as Element | null)?.closest?.(".kindlist, .legend")) listKind = null;
  }}
/>

{#snippet glyph(g: Glyph, acc = false)}
  <svg class="glyph" class:acc viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
    {#if g === "circle" || g === "dot"}
      <circle cx="5" cy="5" r="3.5" class:fill={g === "dot"} />
    {:else if g === "diamond"}
      <path d="M5 0.8 9.2 5 5 9.2 0.8 5Z" />
    {:else if g === "triangle"}
      <path d="M5 1 9 8.4H1Z" />
    {:else}
      <rect x="1.5" y="1.5" width="7" height="7" class:fill={g === "block"} />
    {/if}
  </svg>
{/snippet}

<div class="graph">
  <div class="bar">
    <span class="mono count">{t("kb.graphCount", { nodes: kb.graph.nodes.length, edges: kb.graph.edges.length })}</span>
    <input
      class="gsearch"
      placeholder={t("kb.graphSearch")}
      aria-label={t("kb.graphSearch")}
      bind:value={kb.query}
      onkeydown={(e) => {
        if (e.key === "Enter") void kb.search(kb.query);
        else if (e.key === "Escape" && kb.query) clearQuery();
      }}
    />
    <button class="btn sm" onclick={recenter}>{t("kb.recenter")}</button>
  </div>
  {#if kinds.length}
    <!-- The legend, a row of its own: it wraps when the kinds are many. -->
    <div class="kinds">
      {#each kinds as k (k.kind)}
        <!-- A kind: its nodes as a list, and the graph framed on them (the rail's section, the same choice). -->
        <button
          class="legend"
          class:on={listKind === k.kind}
          aria-expanded={listKind === k.kind}
          title={t("kb.kindOpen", { kind: k.kind })}
          bind:this={legendEls[k.kind]}
          onclick={() => toggleKind(k.kind)}>{@render glyph(glyphOf(k.kind))}{k.kind}<span class="lcount">{k.count}</span></button
        >
      {/each}
    </div>
  {/if}
  <div class="body">
  <div class="stage" bind:this={stageEl} bind:clientWidth={stageW} bind:clientHeight={stageH}>
    {#if !kb.graph.nodes.length}
      <p class="empty">{t("kb.graphEmpty")}</p>
    {/if}
    <canvas bind:this={canvas} onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={() => (drag = null)} onpointerleave={leave} onwheel={wheel}></canvas>
    {#if listKind}
      <div class="kindlist" style="left: {listLeft}px; width: {listW}px" role="dialog" aria-label={t("kb.kindOpen", { kind: listKind })}>
        <div class="klhead">
          {@render glyph(glyphOf(listKind))}
          <span class="mono kllab">{listKind}</span>
          <span class="mono klcount">{rows.length}</span>
          <span class="grow"></span>
          <button class="x" onclick={closeList} aria-label={t("kb.kindClose")} title={t("kb.kindClose")}>×</button>
        </div>
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="klfilter"
          placeholder={t("kb.kindFilter")}
          aria-label={t("kb.kindFilter")}
          bind:value={listFilter}
          autofocus
        />
        <ul class="klrows">
          {#each rows as r (r.id)}
            <li>
              <button class="klrow" class:on={picked?.id === r.id} onclick={() => askAbout(r.id)} title={t("kb.kindAsk")}>
                <span class="kname">{r.name}</span>
                <span class="mono kmeta">{t("kb.kindMeta", { links: r.links, mentions: r.mentions })}</span>
              </button>
            </li>
          {/each}
        </ul>
        {#if !rows.length}<p class="klnone">{t("kb.kindNone")}</p>{/if}
      </div>
    {/if}
    {#if kb.graph.nodes.length}
      <div class="fig mono" aria-hidden="true">
        <span class="fdot"></span>
        <span>{t("kb.graphFig")}</span>
      </div>
    {/if}
    {#if kb.shownQuery || centreNode}
      <div class="boxes" bind:clientHeight={boxesH}>
        {#if centreNode}
          <div class="query" aria-live="polite">
            <div class="qhead">
              <span class="mono qlab">{t("kb.graphFocus")}</span>
              <span class="grow"></span>
              {#each [1, 2] as d (d)}
                <button class="qbtn" class:on={depth === d} aria-pressed={depth === d} onclick={() => (depth = d)}>{t("kb.graphHops", { n: d })}</button>
              {/each}
              <button class="x" onclick={() => (centre = null)} aria-label={t("kb.graphLeaveFocus")} title={t("kb.graphLeaveFocus")}>×</button>
            </div>
            <p class="qtext">{centreNode.name}</p>
            <div class="mono qmeta">{t("kb.graphFocusMeta", { nodes: visible?.size ?? 0 })}</div>
          </div>
        {/if}
        {#if kb.shownQuery}
          <div class="query" aria-live="polite">
            <div class="qhead">
              <span class="mono qlab">{t("kb.graphQuery")}</span>
              <span class="grow"></span>
              {#if queried.size}
                <button class="qbtn" class:on={relatedOnly} aria-pressed={relatedOnly} onclick={() => (relatedOnly = true)}>{t("kb.graphRelatedOnly")}</button>
                <button class="qbtn" class:on={!relatedOnly} aria-pressed={!relatedOnly} onclick={() => (relatedOnly = false)}>{t("kb.graphShowAll")}</button>
              {/if}
              <button class="x" onclick={clearQuery} aria-label={t("kb.graphClearQuery")} title={t("kb.graphClearQuery")}>×</button>
            </div>
            <p class="qtext">{kb.shownQuery}</p>
            <div class="mono qmeta">
              {#if queried.size}
                {t("kb.graphQueryMeta", { sources: querySources, entities: queried.size })}
              {:else}
                {t("kb.graphNoRelated")}
              {/if}
            </div>
          </div>
        {/if}
      </div>
    {/if}
    {#if kb.graph.nodes.length && railOn}
      <nav class="rail" aria-label={t("kb.graphSections")}>
        {#each sections as sec, i (sec.id)}
          <button
            class="stop"
            class:on={section === sec.id}
            class:head={i === 0}
            aria-current={section === sec.id ? "true" : undefined}
            title="{sec.label} · {sec.n}"
            onclick={() => goTo(sec.id)}
          >
            <span class="slab">{sec.label}</span>
            <span class="snum">{String(i).padStart(2, "0")}</span>
            <span class="sline"></span>
          </button>
        {/each}
      </nav>
    {/if}
    {#if picked}
      <aside class="detail" class:folded={card.folded} style="max-height: {cardMaxHeight(stageH, kb.shownQuery || centreNode ? boxesH : 0, BOXES_BOTTOM)}px" bind:clientHeight={cardH}>
        <div class="dhead">
          {#if card.folded}
            <span class="dname small">{@render glyph(glyphOf(picked.kind), true)}<span>{picked.name}</span></span>
            <span class="mono dlab kindtag">{picked.kind}</span>
          {:else}
            <span class="mono dlab">{t("kb.graphEntity")} · {picked.kind}</span>
          {/if}
          <button
            class="x fold"
            onclick={() => flip("folded")}
            aria-expanded={!card.folded}
            aria-label={card.folded ? t("kb.graphUnfoldCard") : t("kb.graphFoldCard")}
            title={card.folded ? t("kb.graphUnfoldCard") : t("kb.graphFoldCard")}>{card.folded ? "▾" : "▴"}</button
          >
          <button class="x" onclick={() => { picked = null; pickedItems = []; kick(); }} aria-label={t("kb.close")}>×</button>
        </div>
        {#if !card.folded}
          <div class="dname">{@render glyph(glyphOf(picked.kind), true)}<span>{picked.name}</span></div>
          {#if picked.description}<p class="desc">{picked.description}</p>{/if}
          {#if centre !== picked.id}
            <button class="btn sm focusbtn" onclick={() => focusOn(picked!.id)}>{t("kb.graphFocusNode")}</button>
          {/if}
          {#if pickedItems.length}
            <button class="srcs mono" onclick={() => flip("sourcesOpen")} aria-expanded={card.sourcesOpen}>
              <span>{card.sourcesOpen ? t("kb.graphHideSources") : t("kb.graphShowSources", { n: pickedItems.length })}</span>
              <span aria-hidden="true">{card.sourcesOpen ? "▴" : "▾"}</span>
            </button>
            {#if card.sourcesOpen}
              {#each pickedItems as item (item.id)}
                <div class="mention">
                  <div class="mtitle">{item.title}</div>
                  <div class="mono msrc">{sourceName(item.source_id)} · {item.line_start}-{item.line_end}</div>
                </div>
              {/each}
            {/if}
          {:else}
            <div class="mlab">{t("kb.mentionedIn", { n: 0 })}</div>
          {/if}
        {/if}
      </aside>
    {/if}
  </div>
  </div>
</div>

<style>
  .graph {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--line);
    /* The legend's row took 33 px of the plate: the smallest plate stays as it was. */
    min-height: 513px;
    /* The page's body, whatever its head took (one row or, narrow, two): no
       guessed head height to keep in step, and the page does not scroll. */
    height: 100%;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--line);
    flex-wrap: wrap;
  }

  .count {
    font-size: 11px;
    color: var(--dim);
    margin-right: 6px;
  }

  .btn.sm {
    height: 26px;
    padding: 0 10px;
  }

  .grow {
    flex: 1;
  }

  /* The bar's second row: the legend alone, wrapping when the kinds are many. */
  .kinds {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 4px;
    padding: 4px 6px;
    border-bottom: 1px solid var(--line);
  }

  .legend {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 6px;
    background: transparent;
    border: 1px solid transparent;
    font-family: var(--mono);
    font-size: 10px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--lab);
  }

  .legend:hover {
    color: var(--hi);
  }

  .legend.on {
    color: var(--hi);
    border-color: var(--kg-acc);
  }

  .lcount {
    color: var(--lab);
    letter-spacing: 0.04em;
  }

  /* A kind's nodes, hanging under its legend entry (left set inline). */
  .kindlist {
    position: absolute;
    top: 6px;
    z-index: 2;
    max-height: min(440px, calc(100% - 18px));
    display: flex;
    flex-direction: column;
    background: var(--card);
    border: 1px solid var(--kg-acc);
  }

  .klhead {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 8px 8px 6px 12px;
  }

  .kllab {
    font-size: 10px;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--kg-acc);
  }

  .klcount {
    font-size: 10px;
    color: var(--lab);
  }

  .klfilter {
    margin: 0 10px 6px;
    height: 26px;
    padding: 0 8px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 12px;
  }

  .klrows {
    list-style: none;
    margin: 0;
    padding: 0 0 6px;
    overflow-y: auto;
    min-height: 0;
  }

  .klrow {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    padding: 5px 12px;
    background: transparent;
    border: 0;
    text-align: left;
    color: var(--txt);
    font-size: 12.5px;
  }

  .klrow:hover,
  .klrow.on {
    background: var(--sel);
    color: var(--hi);
  }

  .klrow.on .kname {
    color: var(--kg-acc);
  }

  .kname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kmeta {
    flex: none;
    font-size: 9.5px;
    color: var(--lab);
  }

  .klnone {
    margin: 0;
    padding: 4px 12px 10px;
    font-size: 12px;
    color: var(--lab);
  }

  .glyph {
    flex-shrink: 0;
    overflow: visible;
  }

  .glyph :global(*) {
    fill: var(--kg-paper);
    stroke: var(--kg-node);
    stroke-width: 1;
  }

  .glyph :global(.fill) {
    fill: var(--kg-node);
  }

  .glyph.acc :global(*) {
    stroke: var(--kg-acc);
  }

  .glyph.acc :global(.fill) {
    fill: var(--kg-acc);
  }

  /* The plate. (The agent panel is a column of the app's layout, beside the page.) */
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .stage {
    position: relative;
    flex: 1;
    min-width: 240px;
    min-height: 0;
    background: var(--kg-paper);
  }



  canvas {
    width: 100%;
    height: 100%;
    display: block;
    cursor: grab;
    touch-action: none;
  }

  /* The plate's caption, top left: FIG. 01 · ENTITY GRAPH · LIVE. */
  .fig {
    position: absolute;
    top: 14px;
    left: 16px;
    display: flex;
    align-items: center;
    gap: 9px;
    font-size: 10px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--body);
    pointer-events: none;
  }

  .fdot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--kg-acc);
  }

  /* The query and focus boxes, bottom left above the coordinates: the search
     the list shows (or a node chosen as centre), in serif italic inside a red
     hairline, what it found, and the switches that change what is shown. */
  .boxes {
    position: absolute;
    left: 16px;
    bottom: 34px;
    width: min(360px, calc(100% - 32px));
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .query {
    padding: 8px 10px 10px 12px;
    background: var(--kg-paper);
    border: 1px solid var(--kg-acc);
  }

  .qhead {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .qbtn {
    height: 20px;
    padding: 0 7px;
    background: transparent;
    border: 1px solid var(--lines);
    color: var(--lab);
    font-family: var(--mono);
    font-size: 9px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .qbtn:hover {
    color: var(--hi);
  }

  .qbtn.on {
    color: var(--kg-acc);
    border-color: var(--kg-acc);
  }

  /* Up to 200 px, down to 80 before the bar wraps: a narrow plate keeps a one-row bar. */
  .gsearch {
    height: 26px;
    flex: 1 1 80px;
    min-width: 80px;
    max-width: 200px;
    padding: 0 8px;
    background: var(--inp);
    border: 1px solid var(--lines);
    color: var(--txt);
    font-family: var(--sans);
    font-size: 12px;
  }

  .focusbtn {
    align-self: flex-start;
  }

  .qlab {
    font-size: 9px;
    letter-spacing: 0.2em;
    text-transform: uppercase;
    color: var(--kg-acc);
  }

  .qtext {
    margin: 4px 0 0;
    font-family: var(--serif);
    font-style: italic;
    font-size: 14px;
    line-height: 1.35;
    color: var(--hi);
    overflow-wrap: anywhere;
  }

  .qmeta {
    margin-top: 6px;
    font-size: 9px;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--lab);
  }

  /* The overview rail, right edge: OVERVIEW 00 ——, then 01, 02… one per kind.
     The chosen stop carries the red rule; the others show their label on hover. */
  .rail {
    position: absolute;
    top: 50%;
    right: 0;
    transform: translateY(-50%);
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 2px;
  }

  .stop {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 22px;
    padding: 0 0 0 10px;
    background: transparent;
    border: 0;
    font-family: var(--mono);
    font-size: 10px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--lab);
  }

  .slab {
    opacity: 0;
    transition: opacity 0.15s;
  }

  .stop:hover .slab,
  .stop:focus-visible .slab,
  .stop.head .slab,
  .stop.on .slab {
    opacity: 1;
  }

  .stop:hover,
  .stop.on {
    color: var(--hi);
  }

  .sline {
    width: 12px;
    height: 1px;
    background: var(--kg-edge);
    transition: width 0.2s;
  }

  .stop:hover .sline {
    width: 20px;
    background: var(--lab);
  }

  .stop.on .sline {
    width: 36px;
    background: var(--kg-acc);
  }

  .empty {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--lab);
    font-size: 13px;
    pointer-events: none;
  }

  /* Top left, under the caption (CARD_TOP, CARD_GAP); its height is capped inline (cardMaxHeight). */
  .detail {
    position: absolute;
    top: 40px;
    left: 12px;
    width: 280px;
    max-width: calc(100% - 24px);
    z-index: 1;
    overflow-y: auto;
    background: var(--card);
    border: 1px solid var(--accln);
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  /* Folded: the header alone, one line. */
  .detail.folded {
    padding: 8px 10px 8px 12px;
  }

  .dname.small {
    flex: 1;
    font-size: 14px;
    gap: 6px;
  }

  .kindtag {
    flex: none;
  }

  .x.fold {
    font-size: 13px;
  }

  /* The passages that mention the entity: a list of its own, closed at first. */
  .srcs {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 0;
    background: transparent;
    border: 0;
    border-top: 1px solid var(--line);
    color: var(--lab);
    font-size: 10px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    text-align: left;
  }

  .srcs:hover {
    color: var(--hi);
  }

  .dhead {
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .dlab {
    flex: 1;
    font-size: 9px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--kg-acc);
  }

  .dname {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    color: var(--hi);
    font-family: var(--serif);
    font-size: 17px;
    line-height: 1.25;
  }

  .dname span {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .x {
    background: transparent;
    border: 0;
    color: var(--lab);
    font-size: 16px;
  }

  .x:hover {
    color: var(--hi);
  }

  .desc {
    margin: 0;
    font-size: 12.5px;
    color: var(--txt);
    line-height: 1.5;
  }

  .mention {
    padding: 6px 0 6px 10px;
    border-top: 1px solid var(--line);
    border-left: 1px solid var(--accln);
  }

  .mtitle {
    font-size: 12.5px;
    color: var(--txt);
  }

  .msrc {
    font-size: 10.5px;
    color: var(--lab);
    margin-top: 2px;
  }
</style>
