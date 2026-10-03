import { invoke, listen, local, bring } from "./ipc.svelte";
import { store, type ArtifactInfo } from "./store.svelte";

/** Mirrors `orchestra_knowledge::Source`: a document in the library. */
export type KSource = {
  id: string;
  source_type: string;
  uri: string;
  status: "pending" | "indexing" | "synced" | "missing" | "duplicate" | "error";
  error: string;
  content_hash: string;
  mtime_ms: number;
  size: number;
  last_synced: number | null;
  topic: string;
  themes: string[];
  extracted: boolean;
  done: number;
  total: number;
  items: number;
  created_at: number;
};

/** Mirrors `knowledge::Listed`: an item, with its score when searched. */
export type KItem = {
  id: number;
  source_id: string;
  chunk_index: number;
  title: string;
  content: string;
  summary: string;
  category: string;
  tags: string[];
  section: string | null;
  line_start: number;
  line_end: number;
  created_at: number;
  score?: number | null;
  match_type?: string | null;
};

export type KNode = { id: number; name: string; kind: string; description: string; mentions: number };
export type KEdge = { source: number; target: number; kind: string };
export type KGraph = { nodes: KNode[]; edges: KEdge[] };
export type KStats = { sources: number; items: number; entities: number; relations: number };
/** Mirrors `store::QueryEntities`: what a search reaches in the graph. */
export type KQueryEntities = { seeds: number[]; related: number[] };

export type KTab = "list" | "graph" | "sources";

/** Mirrors `knowledge::EmbeddingStatus`. */
export type KEmbedding = { enabled: boolean; model: string; embedded: number; failed: number; total: number; error: string };

export {
  K_AGENT,
  K_CONFIG,
  K_POOL,
  K_EXTRACT,
  K_EMBED_ENABLED,
  K_EMBED_URL,
  K_EMBED_MODEL,
  K_EMBED_KEY,
  K_EMBED_DIMS,
  K_EMBED_RATE,
} from "./knowledgeKeys";

/**
 * The knowledge library as the UI sees it: its sources (each also an
 * artifact of kind `knowledge`, for its name, colour and tags), the items
 * listed or found, the entity graph, and counts. Sources change under the
 * core's sync loop and arrive as `knowledge` events.
 */
class Knowledge {
  tab = $state<KTab>("list");
  sources = $state<KSource[]>([]);
  /** Sources that share most of their passages with another, by source id. */
  overlaps = $state<Record<string, { other: string; shared: number; percent: number }>>({});
  items = $state<KItem[]>([]);
  graph = $state<KGraph>({ nodes: [], edges: [] });
  stats = $state<KStats>({ sources: 0, items: 0, entities: 0, relations: 0 });
  embedding = $state<KEmbedding>({ enabled: false, model: "", embedded: 0, failed: 0, total: 0, error: "" });
  query = $state("");
  /** The query the items shown answer ("" for the plain list). */
  shownQuery = $state("");
  /** The entities `shownQuery` reaches: named by it, and within two relations. */
  queryEntities = $state<KQueryEntities>({ seeds: [], related: [] });
  category = $state("");
  sourceFilter = $state("");
  loading = $state(false);
  /** Supported file extensions. */
  formats = $state<string[]>([]);
  /** The file viewer's file, when it is in the library. */
  viewerSource = $state<KSource | null>(null);
  loaded = false;

  /** The artifact listing a source. */
  artifactOf(id: string): ArtifactInfo | undefined {
    return store.artifacts.find((a) => a.id === id);
  }

  /** A source's display name: its artifact's title, else its file name. */
  nameOf(s: KSource): string {
    return this.artifactOf(s.id)?.title ?? s.uri.split(/[\\/]/).at(-1) ?? s.uri;
  }

  get indexing(): KSource[] {
    return this.sources.filter((s) => s.status === "pending" || s.status === "indexing");
  }

  /** Sources fully indexed, of all. */
  get syncedCount(): number {
    return this.sources.filter((s) => s.status === "synced").length;
  }

  async show(tab?: KTab) {
    if (tab) this.tab = tab;
    store.view = "knowledge";
    if (!store.artifacts.length) await store.loadArtifacts();
    await this.load();
  }

  async loadOverlaps() {
    try {
      const list = await invoke<{ source: string; other: string; shared: number; percent: number }[]>("knowledge_overlaps");
      this.overlaps = Object.fromEntries(list.map((o) => [o.source, { other: o.other, shared: o.shared, percent: o.percent }]));
    } catch {
      // Cosmetic.
    }
  }

  async load() {
    try {
      const [sources, stats, formats] = await Promise.all([
        invoke<KSource[]>("knowledge_sources"),
        invoke<KStats>("knowledge_stats"),
        this.formats.length ? Promise.resolve(this.formats) : invoke<string[]>("knowledge_formats"),
      ]);
      this.sources = sources;
      this.stats = stats;
      this.formats = formats;
      void this.loadOverlaps();
      this.loaded = true;
      await this.loadEmbedding();
      await this.refreshTab();
    } catch (err) {
      store.lastError = String(err);
    }
  }

  async refreshTab() {
    if (this.tab === "list") await this.search(this.shownQuery);
    else if (this.tab === "graph") await this.loadGraph();
  }

  async setTab(tab: KTab) {
    this.tab = tab;
    await this.refreshTab();
  }

  /** Only the newest search may land. */
  private searchSeq = 0;

  /** List items, or the matches of `query` when it is not empty. */
  async search(query: string) {
    const seq = ++this.searchSeq;
    this.loading = true;
    try {
      const items = await invoke<KItem[]>("knowledge_items", {
        source: this.sourceFilter || null,
        query: query.trim() || null,
      });
      if (seq !== this.searchSeq) return;
      this.items = items;
      this.shownQuery = query.trim();
      void this.loadQueryEntities(seq);
    } catch (err) {
      if (seq === this.searchSeq) store.lastError = String(err);
    } finally {
      if (seq === this.searchSeq) this.loading = false;
    }
  }

  /** What the shown query reaches in the graph, for the graph view to light. */
  private async loadQueryEntities(seq: number) {
    const query = this.shownQuery;
    let found: KQueryEntities = { seeds: [], related: [] };
    if (query) {
      try {
        found = await invoke<KQueryEntities>("knowledge_query_entities", { query });
      } catch {
        // Cosmetic: the graph shows the query unlit.
      }
    }
    if (seq === this.searchSeq) this.queryEntities = found;
  }

  async loadGraph() {
    try {
      this.graph = await invoke<KGraph>("knowledge_graph");
    } catch (err) {
      store.lastError = String(err);
    }
  }

  async entityItems(id: number): Promise<KItem[]> {
    try {
      return await invoke<KItem[]>("knowledge_entity_items", { id });
    } catch (err) {
      store.lastError = String(err);
      return [];
    }
  }

  /** Add a file; `track` makes `path` relative to that track's folder. Returns why not, or "". */
  async add(path: string, track?: string): Promise<string> {
    try {
      const added = await invoke<{ source: KSource; artifact: ArtifactInfo }>("knowledge_add", { path, track: track ?? null });
      if (!store.artifacts.some((a) => a.id === added.artifact.id)) store.artifacts = [added.artifact, ...store.artifacts];
      this.upsert(added.source);
      return "";
    } catch (err) {
      return String(err);
    }
  }

  /** Pick files from disk and add each. */
  async pickAndAdd() {
    let paths: string[] = [];
    try {
      paths = await bring(await invoke<string[]>("pick_files", { start: local ? (store.currentTrack?.cwd ?? null) : null }));
    } catch (err) {
      store.lastError = String(err);
      return;
    }
    const refused: string[] = [];
    for (const p of paths) {
      const why = await this.add(p);
      if (why) refused.push(`${p.split(/[\\/]/).at(-1)}: ${why}`);
    }
    if (refused.length) store.lastError = refused.join("\n");
  }

  async sync(id: string) {
    try {
      await invoke("knowledge_sync", { id });
    } catch (err) {
      store.lastError = String(err);
    }
  }

  /** Remove a document: its artifact goes, and the core drops the source. */
  async remove(id: string): Promise<string> {
    const why = await store.deleteArtifact(id);
    if (!why) this.drop(id);
    return why;
  }

  async loadEmbedding() {
    try {
      this.embedding = await invoke<KEmbedding>("knowledge_embedding_status");
    } catch {
      /* the banner keeps its last word */
    }
  }

  async loadFormats() {
    try {
      this.formats = await invoke<string[]>("knowledge_formats");
    } catch {
      /* the button stays hidden */
    }
  }

  /** Which file `viewerSource` answers for: `track\0path`. */
  viewerKey = $state("");

  /** Whether a file of the workspace panel is in the library. */
  async checkViewer(track: string, path: string) {
    const key = `${track}\0${path}`;
    if (this.viewerKey !== key) {
      // Nothing is known about a new file until its answer lands.
      this.viewerKey = key;
      this.viewerSource = null;
    }
    let found: KSource | null = null;
    try {
      found = await invoke<KSource | null>("knowledge_source_for", { track, path });
    } catch {
      found = null;
    }
    if (this.viewerKey === key) this.viewerSource = found;
  }

  upsert(s: KSource) {
    const i = this.sources.findIndex((x) => x.id === s.id);
    if (i >= 0) this.sources[i] = s;
    else this.sources = [s, ...this.sources];
    if (this.viewerSource && (this.viewerSource.id === s.id || this.viewerSource.uri === s.uri)) this.viewerSource = s;
    this.scheduleRefresh(s.status === "synced");
  }

  drop(id: string) {
    this.sources = this.sources.filter((s) => s.id !== id);
    this.items = this.items.filter((i) => i.source_id !== id);
    if (this.viewerSource?.id === id) this.viewerSource = null;
    this.scheduleRefresh(true);
  }

  private timer: ReturnType<typeof setTimeout> | undefined;
  /** Whether any change since the last refresh brought new content. */
  private contentChanged = false;
  /** Counts (and, once a sync lands, the open tab) follow changes, at most twice a second. */
  private scheduleRefresh(content: boolean) {
    this.contentChanged ||= content;
    clearTimeout(this.timer);
    this.timer = setTimeout(async () => {
      const changed = this.contentChanged;
      this.contentChanged = false;
      try {
        this.stats = await invoke<KStats>("knowledge_stats");
      } catch {
        /* counts can wait */
      }
      await this.loadEmbedding();
      // A source synced or removed changes what the others share.
      if (changed) await this.loadOverlaps();
      if (changed && store.view === "knowledge") await this.refreshTab();
    }, 500);
  }
}

export const kb = new Knowledge();

/** Subscribe once. */
export async function connectKnowledge() {
  await Promise.all([
    listen<KSource>("knowledge", (e) => kb.upsert(e.payload)),
    listen<string>("knowledge-removed", (e) => kb.drop(e.payload)),
    listen("knowledge-embedding", () => kb.loadEmbedding()),
  ]);
}
