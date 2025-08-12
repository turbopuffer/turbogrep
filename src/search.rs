use crate::chunker::Chunk;
use crate::turbopuffer::TurbopufferError;
use crate::{chunker, embeddings, project, sync, turbopuffer, vprintln};
use anyhow::Result;
use embeddings::Embedding;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("Empty query provided")]
    EmptyQuery,
    #[error("No embedding returned for query")]
    NoEmbedding,
    #[error("Namespace not found")]
    NamespaceNotFound,
    #[error("Failed to build index: {0}")]
    IndexBuildFailed(String),
    #[error("Turbopuffer error: {0}")]
    TurbopufferError(#[from] turbopuffer::TurbopufferError),
    #[error("Embedding error: {0}")]
    EmbeddingError(#[from] embeddings::EmbeddingError),
    #[error("Namespace and directory error: {0}")]
    NamespaceError(String),
}

/// Load content from local file for a chunk
fn load_chunk_content(chunk: &mut chunker::Chunk) -> Result<()> {
    let path = Path::new(&chunk.path);
    if !path.exists() {
        return Ok(()); // File no longer exists, leave content as None
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let lines: Vec<String> = reader
        .lines()
        .skip((chunk.start_line - 1) as usize)
        .take((chunk.end_line - chunk.start_line + 1) as usize)
        .collect::<Result<Vec<_>, _>>()?;

    if !lines.is_empty() {
        chunk.content = Some(lines.join("\n"));
    }

    Ok(())
}

/// Convert chunks to ripgrep-style output format for fzf compatibility  
fn chunks_to_ripgrep_format(
    chunks: Vec<chunker::Chunk>,
    root_dir: &str,
    show_scores: bool,
) -> String {
    chunks
        .into_iter()
        .map(|chunk| {
            // Convert absolute path to relative path
            let relative_path = std::path::Path::new(&chunk.path)
                .strip_prefix(root_dir)
                .map(|p| p.to_string_lossy())
                .unwrap_or_else(|_| chunk.path.as_str().into());

            // Use first line of chunk content as preview, or fallback to content summary
            let preview = chunk
                .content
                .as_ref()
                .and_then(|content| content.lines().next())
                .unwrap_or("[no content]")
                .trim();

            if show_scores {
                if let Some(distance) = chunk.distance {
                    format!(
                        "{}:{}:{:.4}:{}",
                        relative_path, chunk.start_line, distance, preview
                    )
                } else {
                    format!("{}:{}:n/a:{}", relative_path, chunk.start_line, preview)
                }
            } else {
                format!("{}:{}:{}", relative_path, chunk.start_line, preview)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub async fn search(
    query: &str,
    directory: &str,
    max_count: usize,
    embedding_concurrency: Option<usize>,
    show_scores: bool,
    regex: bool,
) -> Result<String, SearchError> {
    let (namespace, root_dir) = project::namespace_and_dir(directory)
        .map_err(|e| SearchError::NamespaceError(e.to_string()))?;

    if query.trim().is_empty() {
        return Err(SearchError::EmptyQuery);
    }

    let results = if regex {
        regex_query(query, &namespace, max_count).await?
    } else {
        semantic_query(query, &namespace, max_count, embedding_concurrency).await?
    };

    // Load content from local files
    let mut results_with_content = results;
    for chunk in &mut results_with_content {
        if let Err(_e) = load_chunk_content(chunk) {
            // Failed to load content - chunk will have no content
        }
    }

    Ok(chunks_to_ripgrep_format(
        results_with_content,
        &root_dir,
        show_scores,
    ))
}

async fn semantic_query(
    query: &str,
    namespace: &str,
    max_count: usize,
    embedding_concurrency: Option<usize>,
) -> Result<Vec<Chunk>, SearchError> {
    let prompt = query.to_string();

    let query_chunk = chunker::Chunk {
        content: Some(prompt),
        ..Default::default()
    };

    let instant = std::time::Instant::now();
    let embedding_provider = match embedding_concurrency {
        Some(concurrency) => embeddings::VoyageEmbedding::with_concurrency(concurrency),
        None => embeddings::VoyageEmbedding::new(),
    };
    let embed_result = embedding_provider
        .embed(vec![query_chunk], embeddings::EmbeddingType::Query)
        .await?;
    vprintln!("embedding w/ voyage took: {:.2?}", instant.elapsed());

    let query_vector = embed_result
        .chunks
        .first()
        .and_then(|chunk| chunk.vector.as_ref())
        .ok_or(SearchError::NoEmbedding)?
        .clone();

    let instant = std::time::Instant::now();
    // Search turbopuffer using existing query_chunks
    let results = turbopuffer::query_chunks(
        &namespace,
        serde_json::json!(["vector", "ANN", query_vector]),
        max_count as u32,
        None,
    )
    .await?;
    vprintln!("tpuf search took: {:.2?}", instant.elapsed());

    Ok(results)
}

async fn regex_query(
    query: &str,
    namespace: &str,
    max_count: usize,
) -> Result<Vec<Chunk>, SearchError> {
    let instant = std::time::Instant::now();
    let results = turbopuffer::query_chunks(
        &namespace,
        serde_json::json!(["id", "asc"]),
        max_count as u32,
        Some(serde_json::json!(["Regex", query])),
    )
    .await?;
    vprintln!("tpuf search took: {:.2?}", instant.elapsed());

    Ok(results)
}

/// Implements a speculative search pattern that races a search against an index sync.
/// This improves perceived performance by returning search results as quickly as possible,
/// while ensuring the index is kept up-to-date in the background.
pub async fn speculate_search(
    query: &str,
    directory: &str,
    max_count: usize,
    embedding_concurrency: Option<usize>,
    show_scores: bool,
    regex: bool,
) -> Result<String, SearchError> {
    loop {
        let mut search_task = tokio::spawn({
            let query = query.to_string();
            let directory = directory.to_string();
            async move {
                search(
                    &query,
                    &directory,
                    max_count,
                    embedding_concurrency,
                    show_scores,
                    regex,
                )
                .await
            }
        });
        let mut index_task = tokio::spawn({
            let directory = directory.to_string();
            async move { sync::tpuf_sync(&directory, embedding_concurrency, regex).await }
        });

        tokio::select! {
            search_result = &mut search_task => {
                match search_result {
                    Ok(Ok(results)) => {
                        // Search succeeded, wait for index to complete and return results
                        match index_task.await {
                            Ok(Ok(_)) => return Ok(results),
                            Ok(Err(_index_err)) => {
                                return Ok(results);
                            }
                            Err(_join_err) => {
                                return Ok(results);
                            }
                        }
                    }
                    Ok(Err(search_err)) => {
                        // Search failed - check if it's because namespace doesn't exist
                        match &search_err {
                            SearchError::TurbopufferError(turbopuffer::TurbopufferError::NamespaceNotFound(_)) => {
                                search_task.abort();
                                match index_task.await {
                                    Ok(Ok(_)) => continue, // Retry search
                                    Ok(Err(index_err)) => return Err(SearchError::IndexBuildFailed(index_err.to_string())),
                                    Err(join_err) => return Err(SearchError::IndexBuildFailed(join_err.to_string())),
                                }
                            }
                            _ => {
                                // Other search error, wait for index and return error
                                match index_task.await {
                                    Ok(_) => return Err(search_err),
                                    Err(_join_err) => {
                                        return Err(search_err);
                                    }
                                }
                            }
                        }
                    }
                    Err(join_err) => {
                        match index_task.await {
                            Ok(_) => return Err(SearchError::IndexBuildFailed(join_err.to_string())),
                            Err(_index_join_err) => {
                                return Err(SearchError::IndexBuildFailed(join_err.to_string()));
                            }
                        }
                    }
                }
            }
            index_result = &mut index_task => {
                match index_result {
                    Ok(Ok(content_changed)) => {
                        if content_changed {
                            search_task.abort();
                            continue; // Retry with updated index
                        }
                        // Index unchanged, wait for search result
                        match search_task.await {
                            Ok(Ok(results)) => return Ok(results),
                            Ok(Err(search_err)) => return Err(search_err),
                            Err(join_err) => return Err(SearchError::IndexBuildFailed(join_err.to_string())),
                        }
                    }
                    Ok(Err(index_err)) => {
                        search_task.abort();
                        return Err(SearchError::IndexBuildFailed(index_err.to_string()));
                    }
                    Err(join_err) => {
                        search_task.abort();
                        return Err(SearchError::IndexBuildFailed(join_err.to_string()));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunks_to_ripgrep_format() {
        let chunks = vec![chunker::Chunk {
            id: 1,
            vector: None,
            path: "/project/src/main.rs".to_string(),
            start_line: 10,
            end_line: 15,
            file_hash: 123,
            chunk_hash: 456,
            file_mtime: 1000,
            file_ctime: 1000,
            content: Some("fn main() {\n    println!(\"Hello!\");\n}".to_string()),
            distance: None,
        }];

        let result = chunks_to_ripgrep_format(chunks, "/project", false);
        let expected = "src/main.rs:10:fn main() {";

        assert_eq!(result, expected);
    }

    #[test]
    fn test_search_error_display() {
        let error = SearchError::EmptyQuery;
        assert_eq!(error.to_string(), "Empty query provided");

        let error = SearchError::NoEmbedding;
        assert_eq!(error.to_string(), "No embedding returned for query");

        let error = SearchError::IndexBuildFailed("test error".to_string());
        assert_eq!(error.to_string(), "Failed to build index: test error");

        let error = SearchError::NamespaceError("test namespace error".to_string());
        assert_eq!(
            error.to_string(),
            "Namespace and directory error: test namespace error"
        );
    }
}
