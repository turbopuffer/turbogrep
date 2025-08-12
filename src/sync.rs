use crate::chunker::Chunk;
use crate::embeddings::Embedding;
use crate::progress::tg_progress_bar;
use crate::{chunker, embeddings, is_verbose, project, turbopuffer, vprintln};

use anyhow::Result;
use futures::stream::{self, StreamExt};

pub fn tpuf_chunk_diff(
    local_chunks: Vec<Chunk>,
    server_chunks: Vec<Chunk>,
) -> Result<(Vec<Chunk>, Vec<Chunk>)> {
    // With file_hash now part of chunk ID, sync logic is much simpler:
    // Any file change will cause all chunk IDs from that file to change automatically

    let local_chunk_ids: std::collections::HashSet<u64> =
        local_chunks.iter().map(|c| c.id).collect();
    let server_chunk_ids: std::collections::HashSet<u64> =
        server_chunks.iter().map(|c| c.id).collect();

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
    regex: bool,
) -> Result<bool> {
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
    // todo clean up this ~~garbage~~ sub-optimal code structure: no need for an if.
    if !local_chunks_to_upload.is_empty() {
        let total_chunks = local_chunks_to_upload.len();
        let pb = tg_progress_bar(total_chunks as u64);

        // Create a progress-tracking stream
        let pb_clone = pb.clone();
        let chunk_stream = stream::iter(local_chunks_to_upload).inspect(move |_| {
            if verbose {
                pb_clone.inc(1);
            }
        });

        // Stream pipeline: chunks -> embed -> write
        let embedding_provider = match embedding_concurrency {
            Some(concurrency) => embeddings::VoyageEmbedding::with_concurrency(concurrency),
            None => embeddings::VoyageEmbedding::new(),
        };
        let embedded_stream =
            embedding_provider.embed_stream(chunk_stream, embeddings::EmbeddingType::Document);

        // Filter out errors and collect successful chunks
        let successful_chunks = embedded_stream.filter_map(|result| async move {
            match result {
                Ok(chunk) => Some(chunk),
                Err(e) => {
                    eprintln!("<(°!°)> Embedding error: {}", e);
                    None
                }
            }
        });

        // Write all chunks with delete_chunks in the first batch
        turbopuffer::write_chunks(
            namespace,
            regex,
            successful_chunks,
            if remote_chunks_to_delete.is_empty() {
                None
            } else {
                Some(remote_chunks_to_delete)
            },
        )
        .await?;
    } else if !remote_chunks_to_delete.is_empty() {
        // Only deletions, no uploads - use empty stream
        turbopuffer::write_chunks(
            namespace,
            regex,
            stream::empty(),
            Some(remote_chunks_to_delete),
        )
        .await?;
    }

    Ok(true) // Content changed
}

pub async fn tpuf_sync(
    directory: &str,
    embedding_concurrency: Option<usize>,
    regex: bool,
) -> Result<bool> {
    let (namespace, root_dir) = project::namespace_and_dir(directory)?;
    vprintln!("namespace={} dir={}", namespace, root_dir);

    // Run chunk_files and all_server_chunks concurrently
    let (local_chunks_res, remote_chunks_res) =
        tokio::join!(async { chunker::chunk_files(&root_dir) }, async {
            turbopuffer::all_chunks(&namespace).await
        });

    let local_chunks = local_chunks_res?;
    let remote_chunks = remote_chunks_res.unwrap_or_default();

    // Calculate the diff in the thread pool
    let (remote_upload, remote_delete) =
        tokio_rayon::spawn(move || tpuf_chunk_diff(local_chunks, remote_chunks)).await?;

    // Apply the diff
    tpuf_apply_diff(
        &namespace,
        remote_upload,
        remote_delete,
        is_verbose(),
        embedding_concurrency,
        regex,
    )
    .await
}
