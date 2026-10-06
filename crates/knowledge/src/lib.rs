// Copyright 2026 The DIVIXI contributors
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// NOTICE OF MODIFICATION (Apache License 2.0, section 4(b)):
//
// The search-result wording emitted by `format_hits` follows the
// `local_knowledge_search` output of Kiro Crew
// (https://github.com/kirodotdev/KiroCrew), Copyright Amazon.com, Inc. or its
// affiliates, licensed under the Apache License, Version 2.0.
//
// This file has been modified by the DIVIXI contributors: the formatting is
// reimplemented in Rust and the source line carries our own identifiers. The
// rest of the module is the DIVIXI contributors' own; no Python source was
// copied.
//
// See the NOTICE file at the root of this repository.

//! The knowledge library: documents the human chose, split into chunks,
//! described by an LLM (title, summary, category, entities and relations),
//! and searched by keyword and by the entity graph. Modelled on Kiro Crew's
//! knowledge library, without local embeddings.

pub mod chunk;
pub mod documents;
pub mod extract;
pub mod fts;
pub mod read;
pub mod store;

pub use chunk::{chunk_document, Chunk, Shape};
pub use extract::{Extraction, SourceSummary};
pub use store::{FileState, Graph, Hit, Item, KnowledgeDb, Library, NewItem, Source, Stats, ToEmbed};

/// Search results as the agent reads them: each hit's title, where it comes
/// from (file, section, lines, and its library when there is more than one),
/// and its text. After Kiro Crew's `local_knowledge_search` output.
pub fn format_hits(hits: &[Hit], libraries: usize) -> String {
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
        if libraries > 1 && !h.library.is_empty() {
            out.push_str(&format!("\n**Library:** {}", h.library));
        }
        out.push_str(&format!("\n**File:** {}\n\n{}", h.source_uri, i.content));
    }
    out
}

/// The library as the agent reads it: one line per source; with more than
/// one library, the sources under each library's name.
pub fn format_sources(sources: &[Source], stats: &Stats, libraries: &[Library]) -> String {
    let mut out = format!(
        "Knowledge library: {} sources, {} items, {} entities, {} relations.",
        stats.sources, stats.items, stats.entities, stats.relations
    );
    let line = |out: &mut String, s: &Source| {
        let name = std::path::Path::new(&s.uri).file_name().and_then(|n| n.to_str()).unwrap_or(&s.uri);
        out.push_str(&format!("\n- {name} — id: {} ({} items, {})", s.id, s.items, s.status));
        if !s.topic.is_empty() {
            out.push_str(&format!(": {}", s.topic));
        }
    };
    if libraries.len() <= 1 {
        for s in sources {
            line(&mut out, s);
        }
        return out;
    }
    out.push_str(&format!(" {} libraries; knowledge_search takes `library` to search one.", libraries.len()));
    for lib in libraries {
        out.push_str(&format!("\n\n## Library: {} ({} sources)", lib.name, lib.sources));
        for s in sources.iter().filter(|s| s.library_id == lib.id) {
            line(&mut out, s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: &str, uri: &str, library: i64) -> Source {
        Source {
            id: id.into(),
            source_type: "local_file".into(),
            uri: uri.into(),
            status: "synced".into(),
            error: String::new(),
            content_hash: String::new(),
            mtime_ms: 0,
            size: 0,
            last_synced: None,
            topic: String::new(),
            themes: vec![],
            extracted: false,
            done: 0,
            total: 0,
            items: 1,
            created_at: 0,
            library_id: library,
        }
    }

    #[test]
    fn sources_are_listed_by_library_only_when_there_are_several() {
        let stats = Stats { sources: 2, items: 2, entities: 0, relations: 0 };
        let sources = [source("ar1", "/d/a.md", 1), source("ar2", "/d/b.md", 2)];
        let one = [Library { id: 1, name: "General".into(), created_at: 0, sources: 2, color: String::new(), tags: vec![] }];
        let text = format_sources(&sources, &stats, &one);
        assert!(!text.contains("Library:"), "{text}");
        assert!(text.contains("- a.md — id: ar1") && text.contains("- b.md — id: ar2"), "{text}");
        let lib = |id: i64, name: &str| Library { id, name: name.into(), created_at: id, sources: 1, color: String::new(), tags: vec![] };
        let two = [lib(1, "General"), lib(2, "Work")];
        let text = format_sources(&sources, &stats, &two);
        let (general, work) = (text.find("## Library: General").unwrap(), text.find("## Library: Work").unwrap());
        let (a, b) = (text.find("a.md").unwrap(), text.find("b.md").unwrap());
        assert!(general < a && a < work && work < b, "each source under its library: {text}");
    }

    #[test]
    fn a_hit_names_its_library_only_when_there_are_several() {
        let item = Item {
            id: 1,
            source_id: "ar1".into(),
            chunk_index: 0,
            title: "Backups".into(),
            content: "nightly".into(),
            summary: String::new(),
            category: "document".into(),
            tags: vec![],
            section: None,
            line_start: 1,
            line_end: 2,
            created_at: 0,
        };
        let hit = Hit { item, score: 1.0, match_type: "keyword".into(), source_uri: "/d/a.md".into(), library: "Work".into() };
        assert!(!format_hits(std::slice::from_ref(&hit), 1).contains("**Library:**"));
        assert!(format_hits(&[hit], 2).contains("**Library:** Work"));
    }
}
