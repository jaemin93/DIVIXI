/** Settings keys, as `knowledge.rs` reads them. */
export const K_AGENT = "knowledge.agent";
export const K_CONFIG = "knowledge.config";
export const K_POOL = "knowledge.pool";
export const K_EXTRACT = "knowledge.extract";
export const K_EMBED_ENABLED = "knowledge.embed.enabled";
export const K_EMBED_URL = "knowledge.embed.url";
export const K_EMBED_MODEL = "knowledge.embed.model";
export const K_EMBED_KEY = "knowledge.embed.key";
export const K_EMBED_DIMS = "knowledge.embed.dims";
export const K_EMBED_RATE = "knowledge.embed.rate";

/** How documents are described: open to every device. */
export const DESCRIBE_KEYS = [K_AGENT, K_CONFIG, K_POOL, K_EXTRACT] as const;

/**
 * The embedding: this PC's window only (`setting_closed` in remote/bridge.rs),
 * since its address is where its key is sent. In the order the pane saves them.
 */
export const EMBED_KEYS = [K_EMBED_URL, K_EMBED_MODEL, K_EMBED_KEY, K_EMBED_DIMS, K_EMBED_RATE, K_EMBED_ENABLED] as const;

/** Whether the embedding is set here: on this PC's own Divixi, not from another device. */
export function embedSettable(local: boolean): boolean {
  return local;
}

/** The keys the knowledge settings pane reads: the embedding's only where it is set. */
export function paneKeys(local: boolean): string[] {
  return embedSettable(local) ? [...DESCRIBE_KEYS, ...EMBED_KEYS] : [...DESCRIBE_KEYS];
}
