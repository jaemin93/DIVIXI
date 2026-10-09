import { invoke, listen, local, bring } from "./ipc.svelte";
import { store, type ArtifactInfo } from "./store.svelte";
import { t } from "./i18n.svelte";
import { displayName, keepLibrary, parseLibrary, sourcesIn, type KLibrary } from "./libraries";

export type { KLibrary } from "./libraries";

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
  /** The library it is in (one only). */
  library_id: number;
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
  /** Every document, of every library. */
  sources = $state<KSource[]>([]);
  /** The libraries, General first. */
  libraries = $state<KLibrary[]>([]);
  /** The library the page shows; null only before the libraries are loaded. */
  library = $state<number | null>(null);
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

  /** The documents of the library shown. */
  get shown(): KSource[] {
    return sourcesIn(this.sources, this.library);
  }

  /** The library shown, or undefined before the libraries are loaded. */
  get current(): KLibrary | undefined {
    return this.libraries.find((l) => l.id === this.library);
  }

  /** A library's name as shown (General in the person's language until renamed). */
  libraryName(l: Pick<KLibrary, "id" | "name">): string {
    return displayName(l, t("kb.lib.general"));
  }

  get indexing(): KSource[] {
    return this.sources.filter((s) => s.status === "pending" || s.status === "indexing");
  }

  /** Sources fully indexed, of those shown. */
  get syncedCount(): number {
    return this.shown.filter((s) => s.status === "synced").length;
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
      const [sources, libraries, formats] = await Promise.all([
        invoke<KSource[]>("knowledge_sources"),
        invoke<KLibrary[]>("knowledge_libraries"),
        this.formats.length ? Promise.resolve(this.formats) : invoke<string[]>("knowledge_formats"),
      ]);
      this.sources = sources;
      this.libraries = libraries;
      // The first time, the library shown last; after, the one shown if it is still there (else General).
      this.setShownLibrary(keepLibrary(this.loaded ? this.library : parseLibrary(store.kbLibrarySaved), libraries));
      await this.loadStats();
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

  /** The library shown, without loading anything: what the graph conversation is told follows it. */
  private setShownLibrary(library: number | null) {
    this.library = library;
    store.graphLibrary = library;
    // A document of another library is no filter here.
    if (this.sourceFilter && !this.shown.some((s) => s.id === this.sourceFilter)) this.sourceFilter = "";
  }

  /** Show one library: the tabs, counts and graph follow; remembered. */
  async showLibrary(library: number) {
    if (library === this.library) return;
    this.setShownLibrary(library);
    void store.rememberKbLibrary(String(library));
    await this.loadStats();
    await this.refreshTab();
  }

  async loadStats() {
    const library = this.library;
    try {
      const stats = await invoke<KStats>("knowledge_stats", { library });
      // Another library picked meanwhile: its own counts are on the way.
      if (library === this.library) this.stats = stats;
    } catch {
      /* counts can wait */
    }
  }

  async loadLibraries() {
    try {
      this.libraries = await invoke<KLibrary[]>("knowledge_libraries");
    } catch (err) {
      store.lastError = String(err);
      return;
    }
    const keep = keepLibrary(this.library, this.libraries);
    if (keep !== this.library) {
      // Deleted (here or from another window): back to General.
      this.setShownLibrary(keep);
      void store.rememberKbLibrary(keep === null ? "" : String(keep));
      await this.loadStats();
      if (store.view === "knowledge") await this.refreshTab();
    }
  }

  /** Make a library and show it. Returns why not, or "". */
  async createLibrary(name: string): Promise<string> {
    try {
      const made = await invoke<KLibrary>("knowledge_library_create", { name });
      await this.loadLibraries();
      await this.showLibrary(made.id);
      return "";
    } catch (err) {
      return String(err);
    }
  }

  /** Returns why not, or "". */
  async renameLibrary(id: number, name: string): Promise<string> {
    try {
      await invoke<KLibrary>("knowledge_library_rename", { id, name });
      await this.loadLibraries();
      return "";
    } catch (err) {
      return String(err);
    }
  }

  /** A library's colour and/or tags, as a design's are set. */
  async updateLibrary(id: number, look: { color?: string; tags?: string[] }) {
    try {
      await invoke<KLibrary>("knowledge_library_update", { id, color: look.color ?? null, tags: look.tags ?? null });
      await this.loadLibraries();
    } catch (err) {
      store.lastError = String(err);
    }
  }

  /** Delete an empty library (not General). Returns why not, or "". */
  async deleteLibrary(id: number): Promise<string> {
    try {
      await invoke("knowledge_library_delete", { id });
    } catch (err) {
      return String(err);
    }
    // The library shown gone: loadLibraries falls back to General.
    await this.loadLibraries();
    return "";
  }

  /** Move a document to another library. */
  async move(id: string, library: number) {
    try {
      const moved = await invoke<KSource>("knowledge_move", { id, library });
      this.upsert(moved);
      await this.loadLibraries();
    } catch (err) {
      store.lastError = String(err);
    }
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
        library: this.library,
      });
      if (seq !== this.searchSeq) return;
      // What the query reaches lands with the query itself: the graph's focus
      // then moves once, not to a stale set and again a moment later.
      const reached = await this.queryEntitiesOf(query.trim());
      if (seq !== this.searchSeq) return;
      this.items = items;
      this.shownQuery = query.trim();
      this.queryEntities = reached;
    } catch (err) {
      if (seq === this.searchSeq) store.lastError = String(err);
    } finally {
      if (seq === this.searchSeq) this.loading = false;
    }
  }

  /** What a query reaches in the graph, for the graph view to light and focus on. */
  private async queryEntitiesOf(query: string): Promise<KQueryEntities> {
    if (!query) return { seeds: [], related: [] };
    try {
      return await invoke<KQueryEntities>("knowledge_query_entities", { query });
    } catch {
      // Cosmetic: the graph shows the query unlit, and whole.
      return { seeds: [], related: [] };
    }
  }

  async loadGraph() {
    const library = this.library;
    try {
      const graph = await invoke<KGraph>("knowledge_graph", { library });
      // Another library picked meanwhile: its own graph is on the way.
      if (library === this.library) this.graph = graph;
    } catch (err) {
      store.lastError = String(err);
    }
  }

  async entityItems(id: number): Promise<KItem[]> {
    try {
      return await invoke<KItem[]>("knowledge_entity_items", { id, library: this.library });
    } catch (err) {
      store.lastError = String(err);
      return [];
    }
  }

  /**
   * Add a file to the library shown (with none, the core picks the first,
   * or makes General again when there is none at all); `track`
   * makes `path` relative to that track's folder. Returns why not, or "".
   */
  async add(path: string, track?: string): Promise<string> {
    try {
      const added = await invoke<{ source: KSource; artifact: ArtifactInfo }>("knowledge_add", { path, track: track ?? null, library: this.library });
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
      await this.loadStats();
      await this.loadEmbedding();
      // A source synced or removed changes what the others share.
      if (changed) await this.loadOverlaps();
      if (changed && store.view === "knowledge") await this.refreshTab();
    }, 500);
  }
}

export const kb = new Knowledge();

// A tag taken out of the vocabulary leaves the libraries too, as it leaves tracks and designs.
store.tagHolders.push(async (name) => {
  for (const l of kb.libraries) if (l.tags.includes(name)) await kb.updateLibrary(l.id, { tags: l.tags.filter((x) => x !== name) });
});

/** Subscribe once. */
export async function connectKnowledge() {
  await Promise.all([
    listen<KSource>("knowledge", (e) => kb.upsert(e.payload)),
    listen<string>("knowledge-removed", (e) => kb.drop(e.payload)),
    listen("knowledge-embedding", () => kb.loadEmbedding()),
    // Made, renamed, deleted, a document moved: here or in another window.
    listen("knowledge-libraries", () => {
      if (kb.loaded) void kb.loadLibraries();
    }),
  ]);
}
