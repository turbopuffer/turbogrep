use crate::chunker::Chunk;
use crate::embeddings::Embedding;
use crate::progress::tg_progress_bar;
use crate::{chunker, embeddings, is_verbose, project, turbopuffer, vprintln};

use anyhow::Result;
use futures::stream::{self, StreamExt};

struct IndexStats {
    chunk_count: usize,
    id_bytes: usize,
    path_bytes: usize,
    start_line_bytes: usize,
    end_line_bytes: usize,
    file_hash_bytes: usize,
    chunk_hash_bytes: usize,
    file_mtime_bytes: usize,
    file_ctime_bytes: usize,
    content_bytes: usize,
    repo_bytes: usize,
}

fn fmt_bytes(bytes: usize) -> String {
    if bytes >= 1_000_000 {
        format!("{:.2} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.2} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{} B", bytes)
    }
}

impl IndexStats {
    fn from_chunks(chunks: &[Chunk]) -> Self {
        let n = chunks.len();
        IndexStats {
            chunk_count: n,
            id_bytes: n * 8,
            path_bytes: chunks.iter().map(|c| c.path.len()).sum(),
            start_line_bytes: n * 4,
            end_line_bytes: n * 4,
            file_hash_bytes: n * 8,
            chunk_hash_bytes: n * 8,
            file_mtime_bytes: n * 8,
            file_ctime_bytes: n * 8,
            content_bytes: chunks.iter().map(|c| c.content.as_ref().map_or(0, |s| s.len())).sum(),
            repo_bytes: chunks.iter().map(|c| c.repo.len()).sum(),
        }
    }

    fn total_bytes(&self) -> usize {
        self.id_bytes
            + self.path_bytes
            + self.start_line_bytes
            + self.end_line_bytes
            + self.file_hash_bytes
            + self.chunk_hash_bytes
            + self.file_mtime_bytes
            + self.file_ctime_bytes
            + self.content_bytes
            + self.repo_bytes
    }

    fn print(&self) {
        eprintln!("indexed {} chunks:", self.chunk_count);
        eprintln!("  {:<12} {:>12}", "field", "bytes");
        eprintln!("  {:<12} {:>12}", "id", fmt_bytes(self.id_bytes));
        eprintln!("  {:<12} {:>12}", "path", fmt_bytes(self.path_bytes));
        eprintln!("  {:<12} {:>12}", "start_line", fmt_bytes(self.start_line_bytes));
        eprintln!("  {:<12} {:>12}", "end_line", fmt_bytes(self.end_line_bytes));
        eprintln!("  {:<12} {:>12}", "file_hash", fmt_bytes(self.file_hash_bytes));
        eprintln!("  {:<12} {:>12}", "chunk_hash", fmt_bytes(self.chunk_hash_bytes));
        eprintln!("  {:<12} {:>12}", "file_mtime", fmt_bytes(self.file_mtime_bytes));
        eprintln!("  {:<12} {:>12}", "file_ctime", fmt_bytes(self.file_ctime_bytes));
        eprintln!("  {:<12} {:>12}", "content", fmt_bytes(self.content_bytes));
        eprintln!("  {:<12} {:>12}", "repo", fmt_bytes(self.repo_bytes));
        eprintln!("  {:<12} {:>12}", "total", fmt_bytes(self.total_bytes()));
    }
}

pub fn tpuf_chunk_diff(
    local_chunks: Vec<Chunk>,
    server_chunks: Vec<Chunk>,
) -> Result<(Vec<Chunk>, Vec<Chunk>)> {
    // With file_hash now part of chunk ID, sync logic is much simpler:
    // Any file change will cause all chunk IDs from that file to change automatically
    
    let local_chunk_ids: std::collections::HashSet<u64> = local_chunks
        .iter()
        .map(|c| c.id)
        .collect();
    let server_chunk_ids: std::collections::HashSet<u64> = server_chunks
        .iter()
        .map(|c| c.id)
        .collect();

    // Delete any server chunks whose IDs don't exist locally
    // (handles file deletion, file changes, and chunk changes automatically)
    let remote_chunks_to_delete: Vec<Chunk> = server_chunks
        .into_iter()
        .filter(|s| !local_chunk_ids.contains(&s.id))
        .collect();

    // Upload any local chunks whose IDs don't exist on server
    let local_chunks_to_upload: Vec<Chunk> = local_chunks
        .into_iter()
        .filter(|c| !server_chunk_ids.contains(&c.id))
        .collect();

    Ok((local_chunks_to_upload, remote_chunks_to_delete))
}

pub async fn tpuf_apply_diff(
    namespace: &str,
    local_chunks_to_upload: Vec<Chunk>,
    remote_chunks_to_delete: Vec<Chunk>,
    verbose: bool,
    embedding_concurrency: Option<usize>,
    use_native_embeddings: bool,
    model: &str,
    multi: bool,
) -> Result<bool> {
    let remote_chunks_to_delete = if multi { vec![] } else { remote_chunks_to_delete };

    if local_chunks_to_upload.is_empty() && remote_chunks_to_delete.is_empty() {
        vprintln!("<(°O°)> turbopuffer search index up-to-date");
        return Ok(false); // No content changed
    }

    if !remote_chunks_to_delete.is_empty() {
        vprintln!(
            "\\(°O°)/ need to delete {} stale chunks",
            remote_chunks_to_delete.len()
        );
    }
    if !local_chunks_to_upload.is_empty() {
        vprintln!(
            "\\(°O°)/ need to index {} chunks",
            local_chunks_to_upload.len()
        );
        vprintln!("using base64 vector encoding (binary f32)");
    }

    // Simple streaming pipeline
    if !local_chunks_to_upload.is_empty() {
        let total_chunks = local_chunks_to_upload.len();
        let pb = tg_progress_bar(total_chunks as u64);

        let stats = IndexStats::from_chunks(&local_chunks_to_upload);

        // Create a progress-tracking stream
        let pb_clone = pb.clone();
        let chunk_stream = stream::iter(local_chunks_to_upload).inspect(move |chunk| {
            if verbose {
                pb_clone.inc(1);
                let size_bytes = chunk.content.as_ref().map_or(0, |c| c.len());
                eprintln!("chunk {}:{}-{}: {} bytes", chunk.path, chunk.start_line, chunk.end_line, size_bytes);
            }
        });

        if use_native_embeddings {
            // Skip Voyage: turbopuffer embeds the content field natively
            turbopuffer::write_chunks(
                namespace,
                chunk_stream,
                if remote_chunks_to_delete.is_empty() {
                    None
                } else {
                    Some(remote_chunks_to_delete)
                },
                Some(model),
            )
            .await?;
        } else {
            // Stream pipeline: chunks -> Voyage embed -> write
            let embedding_provider = match embedding_concurrency {
                Some(concurrency) => embeddings::VoyageEmbedding::with_concurrency(concurrency),
                None => embeddings::VoyageEmbedding::new(),
            };
            let embedded_stream = embedding_provider
                .embed_stream(chunk_stream, embeddings::EmbeddingType::Document);

            let successful_chunks = embedded_stream.filter_map(|result| async move {
                match result {
                    Ok(chunk) => Some(chunk),
                    Err(e) => {
                        eprintln!("<(°!°)> Embedding error: {}", e);
                        None
                    }
                }
            });

            turbopuffer::write_chunks(
                namespace,
                successful_chunks,
                if remote_chunks_to_delete.is_empty() {
                    None
                } else {
                    Some(remote_chunks_to_delete)
                },
                Some(model),
            )
            .await?;
        }

        stats.print();
    } else if !remote_chunks_to_delete.is_empty() {
        // Only deletions, no uploads - use empty stream
        turbopuffer::write_chunks(namespace, stream::empty(), Some(remote_chunks_to_delete), Some(model))
            .await?;
    }

    Ok(true) // Content changed
}

pub async fn tpuf_sync(directory: &str, embedding_concurrency: Option<usize>, use_native_embeddings: bool, model: &str, namespace_override: Option<&str>, debug_chunks: bool, multi: bool) -> Result<bool> {
    let (derived_namespace, root_dir) = project::namespace_and_dir(directory)?;
    let namespace = namespace_override.map(|s| s.to_string()).unwrap_or(derived_namespace);
    vprintln!("namespace={} dir={}", namespace, root_dir);

    // Run chunk_files and all_server_chunks concurrently
    let (local_chunks_res, remote_chunks_res) = tokio::join!(
        async {
            chunker::chunk_files(&root_dir)
        },
        async {
            turbopuffer::all_chunks(&namespace).await
        }
    );

    let local_chunks = local_chunks_res?;
    let remote_chunks = remote_chunks_res.unwrap_or_default();

    // Calculate the diff in the thread pool
    let (remote_upload, remote_delete) =
        tokio_rayon::spawn(move || tpuf_chunk_diff(local_chunks, remote_chunks)).await?;

    if debug_chunks {
        for chunk in &remote_upload {
            eprintln!("{}:{}-{}", chunk.path, chunk.start_line, chunk.end_line);
            if let Some(content) = &chunk.content {
                eprintln!("{}", content);
            }
            eprintln!();
        }
    }

    // Apply the diff
    tpuf_apply_diff(&namespace, remote_upload, remote_delete, is_verbose(), embedding_concurrency, use_native_embeddings, model, multi).await
}
