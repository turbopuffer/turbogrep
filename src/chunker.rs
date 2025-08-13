use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use ignore::types::{FileTypeDef, TypesBuilder};
use num_cpus;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, Tree};
use xxhash_rust::xxh3::xxh3_64;

/// Extracts function content with preceding comments.
/// Returns the combined text (comments + function) but with minimal allocations.
/// The content includes preceding comments, but metadata should be about the function only.
pub fn extract_function_with_comments<'a>(
    tree: &Tree,
    function_node: Node,
    source: &'a str,
) -> &'a str {
    let function_start_byte = function_node.start_byte();
    let function_end_byte = function_node.end_byte();
    let function_start_line = function_node.start_position().row;

    // Start with just the function
    let mut comment_start_byte = function_start_byte;

    // Check siblings at the parent level (handles both top-level functions and methods in classes)
    let parent = function_node.parent().unwrap_or_else(|| tree.root_node());
    let mut cursor = parent.walk();

    // Collect all sibling nodes
    let mut nodes = Vec::new();
    if cursor.goto_first_child() {
        loop {
            nodes.push((cursor.node(), cursor.node().start_byte()));
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    // Find the function in the list
    if let Some(func_pos) = nodes.iter().position(|(n, _)| n.id() == function_node.id()) {
        // Look backwards from the function for comments
        let mut found_comment_near_function = false;
        let mut last_comment_line = function_start_line;

        for i in (0..func_pos).rev() {
            let (node, start_byte) = &nodes[i];

            if matches!(
                node.kind(),
                "comment"
                    | "line_comment"
                    | "block_comment"
                    | "doc_comment"
                    | "documentation_comment"
            ) {
                let comment_start_line = node.start_position().row;
                let comment_end_line = node.end_position().row;

                // First check: is this comment close to the function?
                if !found_comment_near_function && function_start_line <= comment_end_line + 2 {
                    found_comment_near_function = true;
                    comment_start_byte = *start_byte;
                    last_comment_line = comment_start_line;
                    continue;
                }

                // Continue including comments that are part of a contiguous block
                if found_comment_near_function && last_comment_line <= comment_end_line + 2 {
                    comment_start_byte = *start_byte;
                    last_comment_line = comment_start_line;
                    continue;
                }
            }

            // Stop if we've found comments and hit a non-comment or a gap
            if found_comment_near_function {
                break;
            }
        }
    }

    // Return the slice from first comment to function end
    let result = &source[comment_start_byte..function_end_byte];

    result
}

/// Markdown specific extraction that keeps the surrounding header in context of each paragraph chunk
fn extract_paragraph_with_heading<'a>(
    paragraph_node: Node,
    source: &'a str,
) -> Option<Cow<'a, str>> {
    let paragraph_start_byte = paragraph_node.start_byte();
    let paragraph_end_byte = paragraph_node.end_byte();
    let paragraph_start_line = paragraph_node.start_position().row;

    // Walk up the tree to find all nodes that could contain headings
    let mut search_contexts = Vec::new();
    let mut current_node = paragraph_node;

    // Walk up the ancestry to collect all potential contexts
    while let Some(parent) = current_node.parent() {
        search_contexts.push(parent);
        current_node = parent;

        if parent.kind() == "list" {
            return None;
        }

        // Stop at document root
        if parent.kind() == "document" {
            break;
        }
    }

    // Find the closest heading before this paragraph
    let mut best_heading: Option<Node> = None;
    let mut closest_distance = usize::MAX;

    for &context in &search_contexts {
        let mut cursor = context.walk();
        if cursor.goto_first_child() {
            loop {
                let node = cursor.node();
                if matches!(node.kind(), "atx_heading" | "setext_heading") {
                    let heading_line = node.start_position().row;
                    if heading_line < paragraph_start_line {
                        let distance = paragraph_start_line - heading_line;
                        if distance < closest_distance {
                            best_heading = Some(node);
                            closest_distance = distance;
                        }
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        if best_heading.is_some() {
            break;
        }
    }

    if let Some(heading) = best_heading {
        let heading_start_byte = heading.start_byte();
        let heading_end_byte = heading.end_byte();
        let x = [
            &source[heading_start_byte..heading_end_byte],
            &source[paragraph_start_byte..paragraph_end_byte],
        ]
        .join("\n");
        Some(Cow::Owned(x))
    } else {
        Some(Cow::Borrowed(
            &source[paragraph_start_byte..paragraph_end_byte],
        ))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("Unsupported file extension: {0}")]
    UnsupportedExtension(String),
    #[error("Parse error: {0}")]
    ParseFailed(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Chunk {
    pub id: u64, // xxhash of "path:start_line:end_line:chunk_hash"
    pub vector: Option<Vec<f32>>,
    // TODO: should be obfuscated for prod, we don't want to store paths
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub file_hash: u64,  // xxhash of file content
    pub chunk_hash: u64, // xxhash of chunk content
    pub file_mtime: u64, // File modification time (Unix timestamp)
    pub file_ctime: u64, // File creation time (Unix timestamp)
    // Content is kept locally but not stored on server for privacy
    pub content: Option<String>,
    // Distance score from similarity search (lower is better, None if not from search)
    #[serde(rename = "$dist")]
    pub distance: Option<f64>,
}

struct FiletypeMatcher {
    glob_set: GlobSet,
    index_to_def: Vec<FileTypeDef>,
}

impl FiletypeMatcher {
    fn detect_language(&self, path: &Path) -> Option<(&'static str, Language, &'static str)> {
        let filename = path.file_name()?.to_str()?;
        let matches = self.glob_set.matches(filename).into_iter();

        // Check matches in order of precedence (last match wins, like ripgrep)
        for match_idx in matches.rev() {
            let def = &self.index_to_def[match_idx];

            match def.name() {
                "rust" => {
                    return Some((
                        "rust",
                        tree_sitter_rust::LANGUAGE.into(),
                        r#"
                        (function_item) @function
                        (struct_item) @function
                        (impl_item) @function
                        "#,
                    ));
                }
                // The default definitio holds multiple definitions for shorthands, we don't really
                // know which one wins.
                "py" | "python" => {
                    return Some((
                        "python",
                        tree_sitter_python::LANGUAGE.into(),
                        r#"
                        (function_definition) @function
                        "#,
                    ));
                }
                "js" => {
                    return Some((
                        "js",
                        tree_sitter_javascript::LANGUAGE.into(),
                        r#"
                        (function_declaration) @function
                        (function_expression) @function
                        "#,
                    ));
                }
                "ts" | "typescript" => {
                    return Some((
                        "ts",
                        tree_sitter_typescript::LANGUAGE_TSX.into(),
                        r#"
                        (function_declaration) @function
                        (function_expression) @function
                        "#,
                    ));
                }
                "go" => {
                    return Some((
                        "go",
                        tree_sitter_go::LANGUAGE.into(),
                        r#"
                        (function_declaration) @function
                        (method_declaration) @function
                        "#,
                    ));
                }
                "java" => {
                    return Some((
                        "java",
                        tree_sitter_java::LANGUAGE.into(),
                        "(method_declaration) @function",
                    ));
                }
                "c" => {
                    return Some((
                        "c",
                        tree_sitter_c::LANGUAGE.into(),
                        "(function_definition) @function",
                    ));
                }
                "cpp" => {
                    return Some((
                        "cpp",
                        tree_sitter_cpp::LANGUAGE.into(),
                        "(function_definition) @function",
                    ));
                }
                "ruby" => {
                    return Some((
                        "ruby",
                        tree_sitter_ruby::LANGUAGE.into(),
                        r#"
                        (method) @function
                        (singleton_method) @function
                        "#,
                    ));
                }
                "bash" | "sh" => {
                    return Some((
                        "bash",
                        tree_sitter_bash::LANGUAGE.into(),
                        "(function_definition) @function",
                    ));
                }
                "md" | "markdown" => {
                    return Some((
                        "markdown",
                        tree_sitter_md::LANGUAGE.into(),
                        r#"
                        (fenced_code_block) @function
                        (list) @function
                        (paragraph) @function
                        "#,
                    ));
                }
                _ => continue,
            }
        }

        None
    }
}

static FILETYPE_MATCHER: OnceLock<FiletypeMatcher> = OnceLock::new();

// Avoid maintaining our own definition. A bit awkward to get the ones from the `ignore` crate

// but this is much better than maintaining our own.
fn get_filetype_matcher() -> &'static FiletypeMatcher {
    FILETYPE_MATCHER.get_or_init(|| {
        let mut builder = TypesBuilder::new();
        builder.add_defaults();
        let types = builder
            .build()
            .expect("Failed to build ripgrep's file types");

        let mut glob_builder = GlobSetBuilder::new();
        let mut index_to_def = Vec::new();

        for def in types.definitions() {
            for pattern in def.globs() {
                if let Ok(glob) = Glob::new(pattern) {
                    glob_builder.add(glob);
                    index_to_def.push(def.clone());
                }
            }
        }

        let glob_set = glob_builder.build().unwrap();
        FiletypeMatcher {
            glob_set,
            index_to_def,
        }
    })
}

pub fn chunk(
    content: &str,
    file_path: &Path,
    metadata: std::fs::Metadata,
) -> Result<Vec<Chunk>, ChunkError> {
    let (lang_name, language, query_str) = get_filetype_matcher()
        .detect_language(file_path)
        .ok_or_else(|| {
            let ext = file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("no extension");
            ChunkError::UnsupportedExtension(ext.to_string())
        })?;

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|_| ChunkError::ParseFailed(format!("Failed to set {} language", lang_name)))?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| ChunkError::ParseFailed("Failed to parse content".to_string()))?;

    let query = Query::new(&language, query_str)
        .map_err(|e| ChunkError::ParseFailed(format!("Query error: {}", e)))?;

    let mut cursor = QueryCursor::new();
    cursor.set_match_limit(10000); // Prevent runaway matches

    let mut captures = cursor.captures(&query, tree.root_node(), content.as_bytes());

    // Extract file timestamps first (cheaper than hashing)
    let file_mtime = metadata
        .modified()
        .unwrap_or(std::time::UNIX_EPOCH)
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let file_ctime = metadata
        .created()
        .unwrap_or(std::time::UNIX_EPOCH)
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Only calculate file hash if we find chunks (lazy evaluation)
    let file_hash = xxh3_64(content.as_bytes());

    // Use Cow to avoid allocation when possible
    let path_str = file_path.to_string_lossy();

    // Pre-allocate chunks vector with reasonable capacity
    let mut chunks = Vec::with_capacity(32); // Most files have < 32 functions

    use tree_sitter::StreamingIterator;
    let mut _function_count = 0;
    while let Some((match_, _)) = captures.next() {
        for capture in match_.captures {
            _function_count += 1;

            // Extract function content with preceding comments
            let function_with_comments = if lang_name == "markdown"
                && (capture.node.kind() == "paragraph" || capture.node.kind() == "list")
            {
                let Some(chunk) = extract_paragraph_with_heading(capture.node, content) else {
                    continue;
                };
                chunk
            } else {
                Cow::Borrowed(extract_function_with_comments(&tree, capture.node, content))
            };

            let start_pos = capture.node.start_position();
            let end_pos = capture.node.end_position();

            // Calculate chunk hash using the full content (including comments)
            let chunk_hash = xxh3_64(function_with_comments.as_bytes());

            // Create ID by hashing path, line numbers, file hash, AND chunk content hash
            // This ensures the ID changes when ANY part of the file changes
            let id = {
                let mut hasher = xxhash_rust::xxh3::Xxh3::new();
                hasher.update(path_str.as_bytes());
                hasher.update(b":");
                hasher.update(&start_pos.row.to_le_bytes()); // Use function line, not comment line
                hasher.update(b":");
                hasher.update(&end_pos.row.to_le_bytes());
                hasher.update(b":");
                hasher.update(&file_hash.to_le_bytes()); // Include file hash
                hasher.update(b":");
                hasher.update(&chunk_hash.to_le_bytes());
                hasher.digest()
            };

            chunks.push(Chunk {
                id,
                vector: None,               // Vector will be set later during embedding
                path: path_str.to_string(), // Only convert to String when storing
                start_line: (start_pos.row + 1) as u32, // Always the function line, not comment line
                end_line: (end_pos.row + 1) as u32, // Always the function line, not comment line
                file_hash,
                chunk_hash,
                file_mtime,
                file_ctime,
                // TODO: chunk() could take ownership of the file str and probably just trim that
                // string to this, to avoid a second allocation.
                content: Some(function_with_comments.to_string()),
                distance: None, // Not from search, so no distance score
            });
        }
    }

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_id_changes_with_content() {
        use std::path::Path;

        // Create mock metadata
        let metadata = std::fs::metadata("Cargo.toml").unwrap();
        let path = Path::new("test.rs");

        // First version of code
        let content1 = r#"fn hello() {
    println!("Hello, world!");
}"#;

        // Second version - same structure, different content, same line count
        let content2 = r#"fn hello() {
    println!("Hello, universe!");
}"#;

        // Chunk both versions
        let chunks1 = chunk(content1, path, metadata.clone()).unwrap();
        let chunks2 = chunk(content2, path, metadata).unwrap();

        // Should have chunks in both cases
        assert!(!chunks1.is_empty(), "First version should produce chunks");
        assert!(!chunks2.is_empty(), "Second version should produce chunks");

        // Chunks should have different IDs because content changed
        let id1 = chunks1[0].id;
        let id2 = chunks2[0].id;
        assert_ne!(
            id1, id2,
            "Chunk IDs should be different when content changes"
        );

        // But same line numbers
        assert_eq!(chunks1[0].start_line, chunks2[0].start_line);
        assert_eq!(chunks1[0].end_line, chunks2[0].end_line);

        // And different chunk hashes
        assert_ne!(chunks1[0].chunk_hash, chunks2[0].chunk_hash);

        // Debug: verify the values are actually different
        // Content 1 and 2 should have different IDs and hashes
    }

    #[test]
    fn test_hash_chunk_files() {
        use std::fs;

        // Create a temporary directory for testing
        let temp_dir = tempfile::tempdir().unwrap();
        let test_dir = temp_dir.path();

        // Create two files with different content
        let file1_path = test_dir.join("test1.rs");
        let file2_path = test_dir.join("test2.rs");

        fs::write(&file1_path, "fn hello() { println!(\"Hello\"); }").unwrap();
        fs::write(&file2_path, "fn world() { println!(\"World\"); }").unwrap();

        // Get hash chunks for the directory
        let hash_chunks = hash_chunk_files(test_dir.to_str().unwrap()).unwrap();

        // Should have 2 chunks (one per file)
        assert_eq!(hash_chunks.len(), 2);

        // The file hashes should be different for different content
        let file1_hash = hash_chunks
            .iter()
            .find(|c| c.path.ends_with("test1.rs"))
            .unwrap()
            .file_hash;
        let file2_hash = hash_chunks
            .iter()
            .find(|c| c.path.ends_with("test2.rs"))
            .unwrap()
            .file_hash;
        assert_ne!(
            file1_hash, file2_hash,
            "Different content should have different hashes"
        );

        // Update file1 with same content as file2
        fs::write(&file1_path, "fn world() { println!(\"World\"); }").unwrap();

        // Get hash chunks again
        let hash_chunks2 = hash_chunk_files(test_dir.to_str().unwrap()).unwrap();

        // Should still have 2 chunks
        assert_eq!(hash_chunks2.len(), 2);

        // Now the hashes should be the same
        let file1_hash2 = hash_chunks2
            .iter()
            .find(|c| c.path.ends_with("test1.rs"))
            .unwrap()
            .file_hash;
        let file2_hash2 = hash_chunks2
            .iter()
            .find(|c| c.path.ends_with("test2.rs"))
            .unwrap()
            .file_hash;
        assert_eq!(
            file1_hash2, file2_hash2,
            "Same content should have same hashes"
        );
    }

    #[test]
    fn test_extract_function_with_comments() {
        let rust_code = r#"use std::collections::HashMap;

/// A helper function to calculate factorial
/// This is a recursive implementation
fn factorial(n: u32) -> u32 {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}

// Process users and return stats
fn process_users(users: Vec<String>) -> HashMap<String, u32> {
    let mut stats = HashMap::new();
    stats
}"#;

        // Parse with tree-sitter
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(rust_code, None).unwrap();

        // Find function nodes
        let query = Query::new(
            &tree_sitter_rust::LANGUAGE.into(),
            "(function_item) @function",
        )
        .unwrap();

        let mut cursor = QueryCursor::new();
        let captures = cursor.captures(&query, tree.root_node(), rust_code.as_bytes());

        use tree_sitter::StreamingIterator;
        let mut captures_iter = captures;

        // Test first function (factorial) with doc comments
        if let Some((match_, _)) = captures_iter.next() {
            let function_node = match_.captures[0].node;
            let content_with_comments =
                extract_function_with_comments(&tree, function_node, rust_code);

            // Should include the doc comments
            assert!(content_with_comments.contains("/// A helper function to calculate factorial"));
            assert!(content_with_comments.contains("/// This is a recursive implementation"));
            assert!(content_with_comments.contains("fn factorial(n: u32) -> u32"));

            // Should not include content from other functions
            assert!(!content_with_comments.contains("process_users"));
        } else {
            panic!("Should find at least one function");
        }

        // Test second function (process_users) with line comment
        if let Some((match_, _)) = captures_iter.next() {
            let function_node = match_.captures[0].node;
            let content_with_comments =
                extract_function_with_comments(&tree, function_node, rust_code);

            // Should include the line comment
            assert!(content_with_comments.contains("// Process users and return stats"));
            assert!(content_with_comments.contains("fn process_users"));

            // Should not include content from the factorial function
            assert!(!content_with_comments.contains("factorial"));
        }
    }

    #[test]
    fn test_extract_method_comments_in_class() {
        let rust_code = r#"struct Calculator {
    value: i32,
}

impl Calculator {
    /// Creates a new calculator
    pub fn new() -> Self {
        Self { value: 0 }
    }

    // Adds a value to the calculator
    pub fn add(&mut self, x: i32) {
        self.value += x;
    }
}"#;

        // Parse with tree-sitter
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(rust_code, None).unwrap();

        // Find function nodes
        let query = Query::new(
            &tree_sitter_rust::LANGUAGE.into(),
            "(function_item) @function",
        )
        .unwrap();

        let mut cursor = QueryCursor::new();
        let captures = cursor.captures(&query, tree.root_node(), rust_code.as_bytes());

        use tree_sitter::StreamingIterator;
        let mut captures_iter = captures;

        // Test first method (new) with doc comments
        if let Some((match_, _)) = captures_iter.next() {
            let function_node = match_.captures[0].node;
            let content_with_comments =
                extract_function_with_comments(&tree, function_node, rust_code);

            // Should include the doc comment
            assert!(
                content_with_comments.contains("/// Creates a new calculator"),
                "Should include doc comment for method inside impl block"
            );
            assert!(content_with_comments.contains("pub fn new() -> Self"));
        } else {
            panic!("Should find at least one method");
        }
    }

    #[test]
    fn test_extract_long_comment_blocks() {
        // Test with Go-style long comment blocks
        let go_code = r#"package main

// buildFiltersForFastPathCheck builds ANDed equality filters between the
// columns in the uniqueness check defined by h.uniqueOrdinals and scalar
// expressions present in a single Values row being inserted. It is expected
// that buildCheckInputScan has been called and has set up in
// uniqueCheckExpr the columns corresponding with the scalars in the
// insert row. buildCheckInputScan has either inlined the insert row as a Values
// expression, or embedded it within a WithScanExpr, in which case `h.mb.inputForInsertExpr`
// holds the input to the WithScanExpr. In the latter case, for a
// given table column ordinal `i` in `h.uniqueOrdinals`, instead of finding the
// matching scalar in the Values row via uniqueCheckCols[i],
// withScanExpr.InCols[i] holds the column ID to match on. scanExpr is
// the scan on the insert target table used on the right side of the semijoins
// in the non-fast-path uniqueness checks, with column ids matching h.scanScope.cols.
//
// The purpose of this function is to build filters representing a uniqueness
// check on a given insert row, which can be applied as a Select from a Scan,
// and optimized during exploration when all placeholders have been filled in.
// The goal is to find a constrained Scan of an index, which consumes all
// filters (meaning it could also be safely executed via a KV lookup in a fast
// path uniqueness check).
func buildFiltersForFastPathCheck(uniqueCheckExpr RelExpr) FiltersExpr {
    return nil
}"#;

        // Parse with tree-sitter
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(go_code, None).unwrap();

        // Find function nodes
        let query = Query::new(
            &tree_sitter_go::LANGUAGE.into(),
            "(function_declaration) @function",
        )
        .unwrap();

        let mut cursor = QueryCursor::new();
        let captures = cursor.captures(&query, tree.root_node(), go_code.as_bytes());

        use tree_sitter::StreamingIterator;
        let mut captures_iter = captures;

        if let Some((match_, _)) = captures_iter.next() {
            let function_node = match_.captures[0].node;
            let content_with_comments =
                extract_function_with_comments(&tree, function_node, go_code);

            // Should include the entire long comment block
            assert!(
                content_with_comments
                    .contains("// buildFiltersForFastPathCheck builds ANDed equality filters"),
                "Should include start of long comment"
            );
            assert!(
                content_with_comments.contains("// path uniqueness check)."),
                "Should include end of long comment"
            );
            assert!(
                content_with_comments.contains("func buildFiltersForFastPathCheck"),
                "Should include function declaration"
            );

            // Count the number of comment lines to ensure we got them all
            let comment_lines: Vec<&str> = content_with_comments
                .lines()
                .filter(|line| line.trim().starts_with("//"))
                .collect();
            assert!(
                comment_lines.len() >= 20,
                "Should include all lines of the long comment block, found: {}",
                comment_lines.len()
            );
        } else {
            panic!("Should find the function");
        }
    }

    #[test]
    fn test_extract_paragraph_with_heading() {
        let markdown_content = r#"# Introduction

This is the introduction paragraph that should include the heading.

## Features

Here are the main features:

- Feature 1
- Feature 2

This paragraph should include the Features heading.

### Sub Feature

This is under a sub-heading.

## Another Section

This paragraph is under another section heading.

Some text without a heading at the start."#;

        // Parse with tree-sitter
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_md::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(markdown_content, None).unwrap();

        // Find paragraph nodes
        let query = Query::new(&tree_sitter_md::LANGUAGE.into(), "(paragraph) @paragraph").unwrap();

        let mut cursor = QueryCursor::new();
        let captures = cursor.captures(&query, tree.root_node(), markdown_content.as_bytes());

        use tree_sitter::StreamingIterator;
        let mut captures_iter = captures;

        // Test first paragraph (should include "# Introduction")
        if let Some((match_, _)) = captures_iter.next() {
            let paragraph_node = match_.captures[0].node;
            let content_with_heading =
                extract_paragraph_with_heading(paragraph_node, markdown_content).unwrap();

            assert!(
                content_with_heading.contains("# Introduction"),
                "Should include the main heading"
            );
            assert!(
                content_with_heading.contains("This is the introduction paragraph"),
                "Should include paragraph content: {content_with_heading:?}"
            );
            assert!(
                !content_with_heading.contains("## Features"),
                "Should not include subsequent sections"
            );
        } else {
            panic!("Should find at least one paragraph");
        }

        // each of the bullets and the paragraph before the bullet list counts as a paragraph
        captures_iter.next();
        captures_iter.next();
        captures_iter.next();

        // Test paragraph under Features section
        if let Some((match_, _)) = captures_iter.next() {
            let paragraph_node = match_.captures[0].node;
            let content_with_heading =
                extract_paragraph_with_heading(paragraph_node, markdown_content).unwrap();

            assert!(
                content_with_heading.contains("## Features"),
                "Should include the Features heading"
            );
            assert!(
                content_with_heading.contains("This paragraph should include the Features heading"),
                "Should include paragraph content: {content_with_heading:?}",
            );
            assert!(
                !content_with_heading.contains("# Introduction"),
                "Should not include earlier sections"
            );
        }

        // Test paragraph under sub-heading
        if let Some((match_, _)) = captures_iter.next() {
            let paragraph_node = match_.captures[0].node;
            let content_with_heading =
                extract_paragraph_with_heading(paragraph_node, markdown_content).unwrap();

            assert!(
                content_with_heading.contains("### Sub Feature"),
                "Should include the sub-heading"
            );
            assert!(
                content_with_heading.contains("This is under a sub-heading"),
                "Should include paragraph content"
            );
        }
    }

    #[test]
    fn test_chunk_test_file() {
        use std::path::Path;

        // Create a simple test file content
        let content = r#"
#[tokio::test]
async fn test_function() {
    // This is a test function
    assert_eq!(1 + 1, 2);
}

fn regular_function() {
    // This is a regular function
    println!("Hello world");
}
"#;

        // Create mock metadata
        let metadata = std::fs::metadata("Cargo.toml").unwrap();
        let path = Path::new("test_file.rs");

        // Try to chunk this content
        let chunks = chunk(content, path, metadata).unwrap();

        println!("Found {} chunks", chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            println!(
                "Chunk {}: lines {}-{}, content: '{}'",
                i,
                chunk.start_line,
                chunk.end_line,
                chunk
                    .content
                    .as_ref()
                    .unwrap_or(&"[no content]".to_string())
            );
        }

        // Should find at least one function
        assert!(!chunks.is_empty(), "Should find at least one function");

        // All chunks should have content
        for chunk in &chunks {
            assert!(chunk.content.is_some(), "All chunks should have content");
            assert!(
                !chunk.content.as_ref().unwrap().is_empty(),
                "Content should not be empty"
            );
        }
    }
}

#[derive(Default)]
pub struct ChunkFileResult {
    pub chunks: Vec<Chunk>,
    pub read_time_ms: u128,
    pub utf_time_ms: u128,
    pub parse_time_ms: u128,
    pub file_size: u64,
}

pub fn chunk_file(path: &Path) -> Result<ChunkFileResult> {
    // Fast path: check file size first to skip empty/huge files
    let metadata = fs::metadata(path)?;
    let file_size = metadata.len();

    // Skip empty files and files larger than 1MB (likely not source code)
    if file_size == 0 || file_size > 1_000_000 {
        return Ok(ChunkFileResult {
            chunks: vec![],
            read_time_ms: 0,
            utf_time_ms: 0,
            parse_time_ms: 0,
            file_size,
        });
    }

    // Time file reading
    let read_instant = Instant::now();
    let content = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            return Ok(ChunkFileResult {
                chunks: vec![],
                file_size,
                ..Default::default()
            });
        }
        Err(e) => return Err(e.into()),
    };
    let read_time = read_instant.elapsed();

    // Fast UTF-8 validation without copying
    let utf_instant = Instant::now();
    let content_str = match std::str::from_utf8(&content) {
        Ok(s) => s,
        Err(_) => {
            return Ok(ChunkFileResult {
                chunks: vec![],
                read_time_ms: 0,
                utf_time_ms: 0,
                parse_time_ms: 0,
                file_size,
            });
        } // Skip binary files
    };
    let utf_time = utf_instant.elapsed();

    // Time parsing
    let parse_instant = Instant::now();
    let chunks = match chunk(content_str, path, metadata) {
        Ok(chunks) => chunks,
        Err(ChunkError::UnsupportedExtension(_)) => vec![],
        Err(e) => return Err(e.into()),
    };
    let parse_time = parse_instant.elapsed();

    Ok(ChunkFileResult {
        chunks,
        read_time_ms: read_time.as_millis(),
        parse_time_ms: parse_time.as_millis(),
        utf_time_ms: utf_time.as_millis(),
        file_size,
    })
}

/// Generic parallel directory walker that processes files and collects chunks
fn parallel_walk_files<F>(
    root_dir: &str,
    use_progress_bar: bool,
    processor: F,
) -> Result<Vec<Chunk>>
where
    F: Fn(&std::path::Path) -> Option<Vec<Chunk>> + Send + Sync + 'static,
{
    let _instant = Instant::now();

    // Shared results collected from all threads
    let all_chunks = Arc::new(Mutex::new(Vec::new()));
    let file_count = Arc::new(Mutex::new(0usize));
    let pb = if use_progress_bar {
        Some(crate::progress::tg_progress_bar(0))
    } else {
        None
    };

    // Wrap the processor in Arc outside the closure
    let processor = Arc::new(processor);

    // Simple parallel directory walking with inline processing
    WalkBuilder::new(root_dir)
        .follow_links(false)
        .hidden(false)
        .threads(num_cpus::get())
        .build_parallel()
        .run(|| {
            let all_chunks = all_chunks.clone();
            let file_count = file_count.clone();
            let filetype_matcher = get_filetype_matcher();
            let pb_clone = pb.clone();
            let processor = processor.clone();

            Box::new(move |result| {
                match result {
                    Ok(entry) if entry.file_type().is_some_and(|ft| ft.is_file()) => {
                        let path = entry.path();
                        if let Some(ref pb) = pb_clone {
                            pb.inc(1);
                        }

                        // Pre-filter by supported file types
                        if filetype_matcher.detect_language(path).is_some() {
                            if let Some(chunks) = processor(path) {
                                if !chunks.is_empty() {
                                    all_chunks.lock().unwrap().extend(chunks);
                                    *file_count.lock().unwrap() += 1;
                                }
                            }
                        }
                    }
                    Ok(_) => {} // Directory or other non-file entry
                    Err(err) => {
                        eprintln!("Error walking directory: {}", err);
                    }
                }
                ignore::WalkState::Continue
            })
        });

    let chunks = Arc::try_unwrap(all_chunks).unwrap().into_inner().unwrap();
    let _files_processed = *file_count.lock().unwrap();

    let _total_time = _instant.elapsed();

    Ok(chunks)
}

pub fn chunk_files(root_dir: &str) -> Result<Vec<Chunk>> {
    parallel_walk_files(root_dir, true, |path| match chunk_file(path) {
        Ok(result) => {
            if !result.chunks.is_empty() {
                Some(result.chunks)
            } else {
                None
            }
        }
        Err(e) => {
            eprintln!("Error processing {}: {}", path.display(), e);
            None
        }
    })
}

/// Create chunks with metadata only (no content) for efficient diffing
/// This is much faster than full chunking since we don't need to parse content
pub fn hash_chunk_files(root_dir: &str) -> Result<Vec<Chunk>> {
    parallel_walk_files(root_dir, false, |path| {
        // Get file content to calculate hash
        match fs::read(path) {
            Ok(content) => {
                let path_str = path.to_string_lossy();
                let file_hash = xxh3_64(&content); // Use actual file content hash
                let metadata = match fs::metadata(path) {
                    Ok(m) => m,
                    Err(_) => return None,
                };
                let file_mtime = metadata
                    .modified()
                    .unwrap_or_else(|_| std::time::SystemTime::now())
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let file_ctime = metadata
                    .created()
                    .unwrap_or_else(|_| std::time::SystemTime::now())
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                // Create a single chunk per file for hash tracking
                let chunk = Chunk {
                    id: file_hash,
                    vector: None,
                    path: path_str.to_string(),
                    start_line: 1,
                    end_line: 1,
                    file_hash,
                    chunk_hash: file_hash, // Use file_hash as chunk_hash for hash chunks
                    file_mtime,
                    file_ctime,
                    content: None,  // No content for hash chunks
                    distance: None, // Not from search, so no distance score
                };

                Some(vec![chunk])
            }
            Err(e) => {
                eprintln!("Error reading file {}: {}", path.display(), e);
                None
            }
        }
    })
}
