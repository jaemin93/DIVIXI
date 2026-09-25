//! Print what the library would index from a file.
//!
//! Run with:
//!   cargo run -p orchestra-knowledge --example extract -- path/to/file.pdf

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("give a file"))?;
    let path = std::path::Path::new(&path);
    let file = orchestra_knowledge::read::read_file(path)?;
    let shape = orchestra_knowledge::read::shape_of(path).unwrap_or(orchestra_knowledge::Shape::Text);
    let chunks = orchestra_knowledge::chunk_document(shape, &file.text);
    println!("{} chars, {} chunks", file.text.chars().count(), chunks.len());
    println!("---\n{}", file.text.chars().take(600).collect::<String>());
    Ok(())
}
