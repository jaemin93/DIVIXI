<script lang="ts">
  import { onDestroy } from "svelte";
  import { kb, type KItem, type KNode } from "./knowledge.svelte";
  import { t } from "./i18n.svelte";

  /**
   * The entity graph, after Kiro Crew's graph view: a small force layout on
   * a canvas, nodes coloured by type and sized by how often they are
   * mentioned. Drag the empty space to pan, the wheel to zoom, a node to
   * move it; a click shows the node and the items that mention it.
   */
  const KIND_COLORS: Record<string, string> = {
    service: "#5b8def",
    technology: "#4caf6a",
    concept: "#a45cf0",
    org: "#e8833a",
    person: "#d9b43b",
    api: "#3fb8c4",
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
        r: 3 + Math.min(9, Math.sqrt(node.mentions) * 2.2),
        pinned: prev?.pinned ?? false,
      };
    });
    const byId = new Map(bodies.map((b) => [b.node.id, b]));
    links = g.edges.flatMap((e) => {
      const a = byId.get(e.source);
      const b = byId.get(e.target);
      return a && b ? [{ a, b }] : [];
    });
    heat = 1;
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
        const f = (900 / d2) * heat;
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
      const f = (d - 70) * 0.02 * heat;
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

  function color(kind: string): string {
    return KIND_COLORS[kind] ?? "#8a8f98";
  }

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
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    ctx.translate(w / 2 + view.x, h / 2 + view.y);
    ctx.scale(view.k, view.k);
    const styles = getComputedStyle(c);
    ctx.strokeStyle = styles.getPropertyValue("--lines") || "#555";
    ctx.lineWidth = 1 / view.k;
    ctx.beginPath();
    for (const { a, b } of links) {
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
    }
    ctx.stroke();
    const label = styles.getPropertyValue("--txt") || "#ccc";
    ctx.font = `${11 / view.k}px ${styles.getPropertyValue("--sans") || "sans-serif"}`;
    for (const b of bodies) {
      ctx.fillStyle = color(b.node.kind);
      ctx.beginPath();
      ctx.arc(b.x, b.y, b.r, 0, Math.PI * 2);
      ctx.fill();
      if (picked?.id === b.node.id) {
        ctx.strokeStyle = label;
        ctx.lineWidth = 2 / view.k;
        ctx.stroke();
        ctx.lineWidth = 1 / view.k;
      }
      if (b.r >= 6 || view.k > 1.4 || picked?.id === b.node.id) {
        ctx.fillStyle = label;
        ctx.fillText(b.node.name, b.x + b.r + 3, b.y + 4 / view.k);
      }
    }
  }

  function loop() {
    if (running) step();
    draw();
    frame = requestAnimationFrame(loop);
  }

  $effect(() => {
    // Rebuild whenever the graph is reloaded.
    void kb.graph;
    build();
  });

  $effect(() => {
    if (!canvas) return;
    frame = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(frame);
  });

  onDestroy(() => cancelAnimationFrame(frame));

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
      const r = Math.max(b.r, 6 / view.k);
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
  }

  function move(e: PointerEvent) {
    if (!drag) return;
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
  }

  async function up() {
    if (!drag) return;
    const { body, moved } = drag;
    drag = null;
    if (body && !moved) {
      body.pinned = false;
      picked = body.node;
      pickedItems = await kb.entityItems(body.node.id);
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
  }

  function recenter() {
    view = { x: 0, y: 0, k: 1 };
    for (const b of bodies) b.pinned = false;
    heat = 1;
  }

  const kinds = $derived([...new Set(kb.graph.nodes.map((n) => n.kind))]);
  const sourceName = (id: string) => {
    const s = kb.sources.find((x) => x.id === id);
    return s ? kb.nameOf(s) : id;
  };
</script>

<div class="graph">
  <div class="bar">
    <span class="mono count">{t("kb.graphCount", { nodes: kb.graph.nodes.length, edges: kb.graph.edges.length })}</span>
    <button class="btn sm" onclick={recenter}>{t("kb.recenter")}</button>
    <button class="btn sm" class:on={running} onclick={() => (running = !running)}>{t("kb.physics")}</button>
    <span class="grow"></span>
    {#each kinds as k (k)}
      <span class="legend"><span class="dot" style="background: {color(k)}"></span>{k}</span>
    {/each}
  </div>
  <div class="stage">
    {#if !kb.graph.nodes.length}
      <p class="empty">{t("kb.graphEmpty")}</p>
    {/if}
    <canvas bind:this={canvas} onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={() => (drag = null)} onwheel={wheel}></canvas>
    {#if picked}
      <aside class="detail">
        <div class="dhead">
          <span class="dot" style="background: {color(picked.kind)}"></span>
          <span class="dname">{picked.name}</span>
          <span class="mono kind">{picked.kind}</span>
          <button class="x" onclick={() => { picked = null; pickedItems = []; }} aria-label={t("kb.close")}>×</button>
        </div>
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
    gap: 5px;
    font-size: 12px;
    color: var(--dim);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
    flex-shrink: 0;
  }

  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
  }

  canvas {
    width: 100%;
    height: 100%;
    display: block;
    cursor: grab;
    touch-action: none;
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
    top: 10px;
    right: 10px;
    width: 280px;
    max-height: calc(100% - 20px);
    overflow-y: auto;
    background: var(--card);
    border: 1px solid var(--lines);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .dhead {
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .dname {
    color: var(--hi);
    font-size: 14px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .kind {
    font-size: 10px;
    color: var(--lab);
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
    padding: 6px 0;
    border-top: 1px solid var(--line);
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
