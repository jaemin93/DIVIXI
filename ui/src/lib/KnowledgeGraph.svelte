<script lang="ts">
  import { kb, type KItem, type KNode } from "./knowledge.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The entity graph, drawn as a plate: a small force layout on a canvas
   * over paper and a faint grid, hairline edges, small markers sized by how
   * often an entity is mentioned. Type is told by the marker's shape, not a
   * colour; the one red goes to the hub (the best-connected entity) and to
   * what is pointed at or picked. Drag the empty space to pan, the wheel to
   * zoom, a node to move it; a click shows the node and the items that
   * mention it.
   */
  type Glyph = "square" | "diamond" | "circle" | "block" | "dot" | "triangle";
  const KIND_GLYPHS: Record<string, Glyph> = {
    service: "square",
    technology: "diamond",
    concept: "circle",
    org: "block",
    person: "dot",
    api: "triangle",
  };

  type Body = { node: KNode; x: number; y: number; vx: number; vy: number; r: number; pinned: boolean };

  let canvas = $state<HTMLCanvasElement>();
  let running = $state(true);
  let picked = $state<KNode | null>(null);
  let pickedItems = $state<KItem[]>([]);

  let bodies: Body[] = [];
  let links: { a: Body; b: Body }[] = [];
  let view = { x: 0, y: 0, k: 1 };
  let frame = 0;
  let heat = 1;

  /** The best-connected entity, drawn as the plate's centre mark. */
  let hub: Body | null = null;
  let hubLinks = 0;
  let hovered: Body | null = null;
  /** Where the pointer is, in world units, for the coordinate readout. */
  let pointer: { x: number; y: number } | null = null;

  function build() {
    const g = kb.graph;
    const old = new Map(bodies.map((b) => [b.node.id, b]));
    bodies = g.nodes.map((node, i) => {
      const prev = old.get(node.id);
      const a = (i / Math.max(1, g.nodes.length)) * Math.PI * 2;
      return {
        node,
        x: prev?.x ?? Math.cos(a) * 120 + (Math.random() - 0.5) * 20,
        y: prev?.y ?? Math.sin(a) * 120 + (Math.random() - 0.5) * 20,
        vx: 0,
        vy: 0,
        r: 2.5 + Math.min(4, Math.sqrt(node.mentions) * 1.1),
        pinned: prev?.pinned ?? false,
      };
    });
    const byId = new Map(bodies.map((b) => [b.node.id, b]));
    links = g.edges.flatMap((e) => {
      const a = byId.get(e.source);
      const b = byId.get(e.target);
      return a && b ? [{ a, b }] : [];
    });
    // Most links, then most mentions; only once there is a web to be the centre of.
    const degree = new Map<Body, number>();
    for (const { a, b } of links) {
      degree.set(a, (degree.get(a) ?? 0) + 1);
      degree.set(b, (degree.get(b) ?? 0) + 1);
    }
    hub = null;
    hubLinks = 0;
    for (const b of bodies) {
      const d = degree.get(b) ?? 0;
      if (d > hubLinks || (d === hubLinks && hub && b.node.mentions > hub.node.mentions)) {
        hub = b;
        hubLinks = d;
      }
    }
    if (hubLinks < 3) hub = null;
    if (hovered && !bodies.includes(hovered)) hovered = null;
    heat = 1;
    kick();
  }

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
    frame = requestAnimationFrame(loop);
  }

  function step() {
    const n = bodies.length;
    for (let i = 0; i < n; i++) {
      const a = bodies[i];
      for (let j = i + 1; j < n; j++) {
        const b = bodies[j];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let d2 = dx * dx + dy * dy;
        if (d2 < 0.01) {
          dx = Math.random() - 0.5;
          dy = Math.random() - 0.5;
          d2 = 0.25;
        }
        if (d2 > 90000) continue;
        const f = (1200 / d2) * heat;
        const d = Math.sqrt(d2);
        a.vx -= (dx / d) * f;
        a.vy -= (dy / d) * f;
        b.vx += (dx / d) * f;
        b.vy += (dy / d) * f;
      }
    }
    for (const { a, b } of links) {
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const d = Math.sqrt(dx * dx + dy * dy) || 1;
      const f = (d - 90) * 0.02 * heat;
      a.vx += (dx / d) * f;
      a.vy += (dy / d) * f;
      b.vx -= (dx / d) * f;
      b.vy -= (dy / d) * f;
    }
    for (const b of bodies) {
      b.vx -= b.x * 0.004 * heat;
      b.vy -= b.y * 0.004 * heat;
      if (b.pinned) {
        b.vx = 0;
        b.vy = 0;
        continue;
      }
      b.vx *= 0.82;
      b.vy *= 0.82;
      b.x += b.vx;
      b.y += b.vy;
    }
    heat = Math.max(0.02, heat * 0.995);
  }

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

    const pick = picked ? bodies.find((b) => b.node.id === picked!.id) : undefined;
    const lit = (b: Body) => b === pick || b === hovered;
    const size = (b: Body) => Math.max(2.5, b.r * Math.min(1.6, Math.max(0.7, k)));

    // The hub's orbit: a dotted ring at the mean distance of its neighbours.
    if (hub) {
      let sum = 0;
      let n = 0;
      for (const { a, b } of links) {
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
    for (const l of links) {
      if (lit(l.a) || lit(l.b)) hot.push(l);
      else if (l.a === hub || l.b === hub) warm.push(l);
      else plain.push(l);
    }
    for (const [set, color, width] of [
      [plain, ink.edge, 0.75],
      [warm, ink.edgeAcc, 0.9],
      [hot, ink.acc, 1],
    ] as const) {
      if (!set.length) continue;
      ctx.strokeStyle = color;
      ctx.lineWidth = width;
      ctx.beginPath();
      for (const { a, b } of set) {
        ctx.moveTo(sx(a.x), sy(a.y));
        ctx.lineTo(sx(b.x), sy(b.y));
      }
      ctx.stroke();
    }

    // Markers: hollow on paper, solid for the filled kinds, red when lit.
    ctx.lineWidth = 1;
    for (const b of bodies) {
      if (b === hub) continue;
      const g = glyphOf(b.node.kind);
      const r = size(b);
      const x = snap(sx(b.x));
      const y = snap(sy(b.y));
      const on = lit(b);
      const solid = g === "block" || g === "dot";
      glyphPath(ctx, g, x, y, r);
      ctx.fillStyle = b === pick ? ink.acc : solid ? (on ? ink.acc : ink.node) : ink.paper;
      ctx.fill();
      ctx.strokeStyle = on ? ink.acc : ink.node;
      ctx.stroke();
      if (b === pick) {
        ctx.strokeStyle = ink.acc;
        ctx.strokeRect(x - r - 4, y - r - 4, (r + 4) * 2, (r + 4) * 2);
      }
    }

    // The hub: a red ring with a dot at its centre.
    if (hub) {
      const x = sx(hub.x);
      const y = sy(hub.y);
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
    }

    // Labels: spaced mono capitals; under the ones that matter, a line of grey meta.
    // Placed in order of importance (what is lit, the hub, the most mentioned), and a
    // label that would land on one already placed is left out rather than overprinted.
    const order = bodies
      .filter((b) => b === hub || lit(b) || b.node.mentions >= 2 || k > 1.4)
      .sort((p, q) => Number(lit(q)) - Number(lit(p)) || Number(q === hub) - Number(p === hub) || q.node.mentions - p.node.mentions);
    // Markers are already on the plate: no label may cover one.
    const taken: [number, number, number, number][] = bodies.map((b) => {
      const r = b === hub ? ringOf(k) : size(b) * 1.3;
      return [sx(b.x) - r, sy(b.y) - r, sx(b.x) + r, sy(b.y) + r];
    });
    const free = (x0: number, y0: number, x1: number, y1: number) =>
      !taken.some(([a0, b0, a1, b1]) => x0 < a1 && x1 > a0 && y0 < b1 && y1 > b0);
    for (const b of order) {
      const on = lit(b);
      const isHub = b === hub;
      const x = sx(b.x);
      const y = sy(b.y);
      if (x < -240 || x > w + 40 || y < -30 || y > h + 30) continue;
      const name = b.node.name.toUpperCase();
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
      if (!on && !free(...box)) continue;
      taken.push(box);
      if (on) {
        // Lit labels sit on a patch of paper so they read over whatever is under them.
        ctx.fillStyle = ink.paper;
        ctx.fillRect(box[0], box[1], box[2] - box[0], box[3] - box[1]);
      }
      ctx.font = `${isHub ? 500 : 400} 10px ${ink.mono}`;
      spaced(ctx, 1.4);
      ctx.fillStyle = on ? ink.acc : ink.label;
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
    if (running) step();
    draw();
    // Keep going while the layout moves or a node is held; otherwise rest until kicked.
    frame = (running && heat > SETTLED) || drag ? requestAnimationFrame(loop) : 0;
  }

  $effect(() => {
    // Rebuild whenever the graph is reloaded.
    void kb.graph;
    build();
  });

  $effect(() => {
    if (!canvas) return;
    kick();
    return () => {
      cancelAnimationFrame(frame);
      frame = 0;
    };
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
    for (let i = bodies.length - 1; i >= 0; i--) {
      const b = bodies[i];
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
      const id = body.node.id;
      picked = body.node;
      pickedItems = [];
      const items = await kb.entityItems(id);
      // Another node may have been picked meanwhile.
      if (picked?.id === id) pickedItems = items;
    } else if (!body && !moved) {
      picked = null;
      pickedItems = [];
    }
  }

  function wheel(e: WheelEvent) {
    e.preventDefault();
    const before = toWorld(e);
    const k = Math.min(4, Math.max(0.25, view.k * Math.exp(-e.deltaY * 0.0015)));
    view.k = k;
    const after = toWorld(e);
    view.x += (after.x - before.x) * k;
    view.y += (after.y - before.y) * k;
    kick();
  }

  function recenter() {
    view = { x: 0, y: 0, k: 1 };
    for (const b of bodies) b.pinned = false;
    heat = 1;
    kick();
  }

  const kinds = $derived([...new Set(kb.graph.nodes.map((n) => n.kind))]);
  const sourceName = (id: string) => {
    const s = kb.sources.find((x) => x.id === id);
    return s ? kb.nameOf(s) : id;
  };
</script>

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
    <button class="btn sm" onclick={recenter}>{t("kb.recenter")}</button>
    <button class="btn sm" class:on={running} onclick={() => {
        running = !running;
        if (running) heat = Math.max(heat, 0.3);
        kick();
      }}>{t("kb.physics")}</button>
    <span class="grow"></span>
    {#each kinds as k (k)}
      <span class="legend">{@render glyph(glyphOf(k))}{k}</span>
    {/each}
  </div>
  <div class="stage">
    {#if !kb.graph.nodes.length}
      <p class="empty">{t("kb.graphEmpty")}</p>
    {/if}
    <canvas bind:this={canvas} onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={() => (drag = null)} onpointerleave={leave} onwheel={wheel}></canvas>
    {#if kb.graph.nodes.length}
      <div class="fig mono" aria-hidden="true">
        <span class="fdot" class:pulse={running}></span>
        <span>{t("kb.graphFig")} · {running ? t("kb.graphLive") : t("kb.graphStill")}</span>
      </div>
    {/if}
    {#if picked}
      <aside class="detail">
        <div class="dhead">
          <span class="mono dlab">{t("kb.graphEntity")} · {picked.kind}</span>
          <button class="x" onclick={() => { picked = null; pickedItems = []; kick(); }} aria-label={t("kb.close")}>×</button>
        </div>
        <div class="dname">{@render glyph(glyphOf(picked.kind), true)}<span>{picked.name}</span></div>
        {#if picked.description}<p class="desc">{picked.description}</p>{/if}
        <div class="mlab">{t("kb.mentionedIn", { n: pickedItems.length })}</div>
        {#each pickedItems as item (item.id)}
          <div class="mention">
            <div class="mtitle">{item.title}</div>
            <div class="mono msrc">{sourceName(item.source_id)} · {item.line_start}-{item.line_end}</div>
          </div>
        {/each}
      </aside>
    {/if}
  </div>
</div>

<style>
  .graph {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--line);
    min-height: 480px;
    height: calc(100vh - 330px);
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

  .btn.on {
    color: var(--hi);
    background: var(--sel);
  }

  .grow {
    flex: 1;
  }

  .legend {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 6px;
    font-family: var(--mono);
    font-size: 10px;
    letter-spacing: 0.12em;
    text-transform: uppercase;
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

  .stage {
    position: relative;
    flex: 1;
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

  .detail {
    position: absolute;
    top: 12px;
    right: 12px;
    width: 280px;
    max-height: calc(100% - 24px);
    overflow-y: auto;
    background: var(--card);
    border: 1px solid var(--accln);
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
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
