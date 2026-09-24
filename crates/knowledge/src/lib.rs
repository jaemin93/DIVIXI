//! The knowledge library: documents the human chose, split into chunks,
//! described by an LLM (title, summary, category, entities and relations),
//! and searched by keyword and by the entity graph. Modelled on Kiro Crew's
//! knowledge library, without local embeddings.

pub mod chunk;
pub mod extract;
pub mod fts;
pub mod read;
pub mod store;

pub use chunk::{chunk_document, Chunk, Shape};
pub use extract::{Extraction, SourceSummary};
pub use store::{FileState, Graph, Hit, Item, KnowledgeDb, NewItem, Source, Stats};

/// Search results as the agent reads them: each hit's title, where it comes
/// from (file, section, lines), and its text. After Kiro Crew's
/// `local_knowledge_search` output.
pub fn format_hits(hits: &[Hit]) -> String {
    if hits.is_empty() {
        return "No relevant knowledge found.".to_string();
    }
    let mut out = String::from("📚 Knowledge Library (supplementary reference — extract only what's relevant to the question):");
    for h in hits {
        let i = &h.item;
        out.push_str("\n\n---\n## ");
        out.push_str(if i.title.is_empty() { "(untitled)" } else { &i.title });
        let name = std::path::Path::new(&h.source_uri).file_name().and_then(|n| n.to_str()).unwrap_or(&h.source_uri);
        out.push_str(&format!("\n**Source:** [{}] {name}", i.source_id));
        if let Some(s) = &i.section {
            out.push_str(&format!(" — {s}"));
        }
        out.push_str(&format!(" (lines {}-{})", i.line_start, i.line_end));
        out.push_str(&format!("\n**File:** {}\n\n{}", h.source_uri, i.content));
    }
    out
}

/// The library as the agent reads it: one line per source.
pub fn format_sources(sources: &[Source], stats: &Stats) -> String {
    let mut out = format!(
        "Knowledge library: {} sources, {} items, {} entities, {} relations.",
        stats.sources, stats.items, stats.entities, stats.relations
    );
    for s in sources {
        let name = std::path::Path::new(&s.uri).file_name().and_then(|n| n.to_str()).unwrap_or(&s.uri);
        out.push_str(&format!("\n- {name} — id: {} ({} items, {})", s.id, s.items, s.status));
        if !s.topic.is_empty() {
            out.push_str(&format!(": {}", s.topic));
        }
    }
    out
}
