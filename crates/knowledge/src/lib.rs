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
pub use store::{FileState, Graph, Hit, Item, KnowledgeDb, NewItem, Source, Stats, ToEmbed};

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
