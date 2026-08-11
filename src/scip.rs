use crate::chunker::Chunk;
use crate::vprintln;
use anyhow::{bail, Context, Result};
use protobuf::Message as _;
use scip::types::{Document, Index as ScipIndex, Occurrence, SymbolRole};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Entry point of the scip-typescript CLI, vendored via `pnpm add -D` in scip-installation-repo.
fn scip_typescript_entrypoint() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/scip-installation-repo/node_modules/@sourcegraph/scip-typescript/dist/src/main.js"
    ))
}

/// Run scip-typescript against `project_dir`, writing `index.scip` into it.
/// Returns the path to the generated index.
pub fn generate_index(project_dir: &Path) -> Result<PathBuf> {
    let entrypoint = scip_typescript_entrypoint();
    if !entrypoint.exists() {
        bail!(
            "scip-typescript not found at {}. Run `pnpm add -D @sourcegraph/scip-typescript` in scip-installation-repo.",
            entrypoint.display()
        );
    }

    vprintln!(
        "<(°○°)> Generating SCIP index for {}",
        project_dir.display()
    );

    let status = Command::new("node")
        .arg(&entrypoint)
        .arg("index")
        .arg("--cwd")
        .arg(project_dir)
        .current_dir(project_dir)
        .status()
        .context("failed to spawn scip-typescript (is Node.js installed?)")?;

    if !status.success() {
        bail!("scip-typescript exited with {status}");
    }

    let index_path = project_dir.join("index.scip");
    if !index_path.exists() {
        bail!(
            "scip-typescript reported success but {} was not created",
            index_path.display()
        );
    }

    vprintln!("<(°◕°)> SCIP index written to {}", index_path.display());

    Ok(index_path)
}

/// Parse a SCIP index file (a protobuf-encoded `Index` message).
fn parse_index(index_path: &Path) -> Result<ScipIndex> {
    let bytes = fs::read(index_path)
        .with_context(|| format!("failed to read {}", index_path.display()))?;
    ScipIndex::parse_from_bytes(&bytes)
        .with_context(|| format!("failed to parse SCIP index {}", index_path.display()))
}

fn is_definition_occurrence(occ: &Occurrence) -> bool {
    occ.symbol_roles & (SymbolRole::Definition as i32) != 0
}

/// 1-based start line of the occurrence's own range (the symbol token itself).
fn occurrence_start_line(occ: &Occurrence) -> Option<u32> {
    occ.range.first().map(|&l| l as u32 + 1)
}

/// 1-based end line of the occurrence's enclosing range (the full definition body),
/// falling back to its own range if no enclosing range was recorded.
fn occurrence_enclosing_end_line(occ: &Occurrence) -> Option<u32> {
    let range = if occ.enclosing_range.len() >= 3 {
        &occ.enclosing_range
    } else {
        &occ.range
    };
    match range.len() {
        4 => Some(range[2] as u32 + 1),
        3 => Some(range[0] as u32 + 1),
        _ => None,
    }
}

/// Find the SCIP symbol that a tree-sitter-derived function chunk defines, by matching the
/// chunk's start line against SCIP definition occurrences in the same document.
///
/// scip-typescript doesn't populate `SymbolInformation.kind`, so we can't ask SCIP "is this
/// symbol a function" directly. Instead we trust tree-sitter's chunk boundaries (which are
/// already scoped to function/method nodes) and look up which symbol was defined at that
/// exact line; when several definitions start on the same line, the one whose enclosing
/// range best matches the chunk's own end line wins.
fn find_defining_symbol(document: &Document, start_line: u32, end_line: u32) -> Option<String> {
    document
        .occurrences
        .iter()
        .filter(|occ| is_definition_occurrence(occ))
        .filter(|occ| occurrence_start_line(occ) == Some(start_line))
        .min_by_key(|occ| {
            let enclosing_end = occurrence_enclosing_end_line(occ).unwrap_or(start_line);
            (enclosing_end as i64 - end_line as i64).abs()
        })
        .map(|occ| occ.symbol.clone())
}

/// Enrich tree-sitter function chunks with SCIP-derived sparse vectors:
/// - `definitions`: `{self_symbol: 1.0}`, the SCIP symbol this chunk defines.
/// - `references`: `{other_symbol: 1.0, ...}`, a bag-of-words of every other symbol
///   referenced within this chunk's line range that is itself a known function chunk.
///
/// Chunks that can't be matched to a SCIP symbol (e.g. anonymous function expressions, or
/// files SCIP didn't index) are returned with `definitions`/`references` left as `None`.
fn enrich_chunks_with_scip(mut chunks: Vec<Chunk>, index: &ScipIndex, project_root: &Path) -> Vec<Chunk> {
    let documents: HashMap<&str, &Document> = index
        .documents
        .iter()
        .map(|d| (d.relative_path.as_str(), d))
        .collect();

    let mut chunks_by_doc: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let rel_path = Path::new(&chunk.path)
            .strip_prefix(project_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| chunk.path.clone());
        if documents.contains_key(rel_path.as_str()) {
            chunks_by_doc.entry(rel_path).or_default().push(i);
        } else {
            vprintln!(
                "<(°◯°)> skipping {}:{}-{} (file not present in the SCIP index)",
                chunk.path,
                chunk.start_line,
                chunk.end_line
            );
        }
    }

    // First pass: find each chunk's defining SCIP symbol, and build the global set of
    // symbols that correspond to one of our indexed function chunks.
    let mut chunk_symbol: HashMap<usize, String> = HashMap::new();
    let mut function_symbols: HashSet<String> = HashSet::new();

    for (rel_path, indices) in &chunks_by_doc {
        let document = documents[rel_path.as_str()];
        for &i in indices {
            let (start_line, end_line) = (chunks[i].start_line, chunks[i].end_line);
            match find_defining_symbol(document, start_line, end_line) {
                Some(symbol) => {
                    function_symbols.insert(symbol.clone());
                    chunk_symbol.insert(i, symbol);
                }
                None => {
                    vprintln!(
                        "<(°◯°)> skipping {}:{}-{} (SCIP indexed this file, but no definition occurrence starts on line {})",
                        chunks[i].path,
                        start_line,
                        end_line,
                        start_line
                    );
                }
            }
        }
    }

    // Second pass: for each matched chunk, collect every non-definition occurrence inside
    // its line range whose symbol is one we know defines a function chunk.
    for (rel_path, indices) in &chunks_by_doc {
        let document = documents[rel_path.as_str()];
        for &i in indices {
            let Some(self_symbol) = chunk_symbol.get(&i).cloned() else {
                continue;
            };
            let (start_line, end_line) = (chunks[i].start_line, chunks[i].end_line);

            let mut references: HashMap<String, f32> = HashMap::new();
            for occ in &document.occurrences {
                if occ.symbol.is_empty() || is_definition_occurrence(occ) {
                    continue;
                }
                if !function_symbols.contains(&occ.symbol) {
                    continue;
                }
                if let Some(line) = occurrence_start_line(occ) {
                    if line >= start_line && line <= end_line {
                        references.insert(occ.symbol.clone(), 1.0);
                    }
                }
            }

            let mut definitions = HashMap::new();
            definitions.insert(self_symbol, 1.0);

            chunks[i].definitions = Some(definitions);
            chunks[i].references = Some(references);
        }
    }

    chunks
}

/// Result of [`index`]: the SCIP-enriched chunks ready to upload, plus how many tree-sitter
/// chunks couldn't be correlated with a SCIP symbol (see `enrich_chunks_with_scip` for the
/// per-chunk reasons, logged via `vprintln` at the point each one is dropped).
pub struct ScipIndexResult {
    pub chunks: Vec<Chunk>,
    pub unmatched: usize,
}

/// Generate a SCIP index for `project_dir` and return the tree-sitter function chunks that
/// SCIP could correlate with a defining symbol, enriched with `definitions`/`references`
/// sparse vectors. Chunks without a match (e.g. anonymous functions) are dropped, since
/// callers upsert these into the same rows as regular indexing and a bare chunk with no
/// SCIP data would only clobber whatever was already there for no benefit.
pub fn index(project_dir: &Path) -> Result<ScipIndexResult> {
    let index_path = generate_index(project_dir)?;

    let root_dir = project_dir.to_string_lossy().to_string();
    let chunks =
        crate::chunker::chunk_files(&root_dir).context("failed to chunk project for SCIP enrichment")?;

    let index = parse_index(&index_path)?;
    let enriched = enrich_chunks_with_scip(chunks, &index, project_dir);
    let total = enriched.len();

    let scip_chunks: Vec<Chunk> = enriched.into_iter().filter(|c| c.definitions.is_some()).collect();
    let unmatched = total - scip_chunks.len();

    vprintln!(
        "<(°◕°)> matched {} of {} function chunks to SCIP symbols ({} unmatched)",
        scip_chunks.len(),
        total,
        unmatched
    );

    Ok(ScipIndexResult {
        chunks: scip_chunks,
        unmatched,
    })
}
