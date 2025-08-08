use crate::chunker::Chunk;
use crate::config::SETTINGS;
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use futures::future::join_all;
use futures::stream::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Instant;

const TURBOPUFFER_REGIONS: &[&str] = &[
    "gcp-us-central1",
    "gcp-us-west1",
    "gcp-us-east4",
    "gcp-northamerica-northeast2",
    "gcp-europe-west3",
    "gcp-asia-southeast1",
    "aws-ap-southeast-2",
    "aws-eu-central-1",
    "aws-us-east-1",
    "aws-us-east-2",
    "aws-us-west-2",
];

static CLIENT: OnceLock<Client> = OnceLock::new();

/// Get a shared HTTP client with optimized configuration
fn get_client() -> &'static Client {
    CLIENT.get_or_init(|| {
        Client::builder()
            .pool_max_idle_per_host(8)
            .pool_idle_timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .http2_keep_alive_interval(Some(std::time::Duration::from_secs(30)))
            .http2_keep_alive_timeout(std::time::Duration::from_secs(10))
            .http2_keep_alive_while_idle(true)
            .brotli(true)
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .tcp_nodelay(true)
            .build()
            .expect("Failed to build HTTP client")
    })
}

pub async fn ping(region: Option<&str>) -> Result<u64, TurbopufferError> {
    let instant = Instant::now();
    let client = get_client();

    let region_to_use = region.unwrap_or_else(|| {
        SETTINGS
            .get()
            .and_then(|s| s.turbopuffer_region.as_deref())
            .unwrap_or("gcp-us-east4")
    });

    let instant = Instant::now();
    let _result = client
        .get(format!("https://{}.turbopuffer.com/", region_to_use))
        .send()
        .await?;
    crate::vprintln!(
        "tpuf ping to {} took {:.2} ms",
        region_to_use,
        instant.elapsed().as_millis()
    );

    let latency = instant.elapsed().as_millis() as u64;
    Ok(latency)
}

pub async fn find_closest_region() -> Result<String, TurbopufferError> {
    let ping_futures: Vec<_> = TURBOPUFFER_REGIONS
        .iter()
        .map(|&region| async move {
            match ping(Some(region)).await {
                Ok(latency) => Some((region.to_string(), latency)),
                Err(_e) => None,
            }
        })
        .collect();

    let results = join_all(ping_futures).await;

    let mut best_region = None;
    let mut best_latency = u64::MAX;

    for result in results {
        if let Some((region, latency)) = result {
            if latency < best_latency {
                best_latency = latency;
                best_region = Some(region);
            }
        }
    }

    match best_region {
        Some(region) => Ok(region),
        None => Ok("gcp-us-east4".to_string()),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TurbopufferError {
    #[error("Missing TURBOPUFFER_API_KEY")]
    MissingApiKey,
    #[error("Request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),
    #[error("Namespace not found: {0}")]
    NamespaceNotFound(String),
    #[error("API error: {0}")]
    ApiError(String),
    #[error("Join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct Performance {
    server_total_ms: u64,
}

#[derive(Deserialize)]
struct QueryResponse {
    rows: Vec<Chunk>,
    performance: Performance,
}

const USE_BASE64_VECTORS: bool = true;

fn vector_to_base64(vector: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for &f in vector {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    general_purpose::STANDARD.encode(&bytes)
}

#[derive(Serialize)]
struct ChunkForUpload {
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    vector: Option<serde_json::Value>,
    path: String,
    start_line: u32,
    end_line: u32,
    file_hash: u64,
    chunk_hash: u64,
    file_mtime: u64,
    file_ctime: u64,
}

impl From<Chunk> for ChunkForUpload {
    fn from(chunk: Chunk) -> Self {
        let vector = if let Some(vec) = chunk.vector {
            if USE_BASE64_VECTORS {
                Some(serde_json::Value::String(vector_to_base64(&vec)))
            } else {
                Some(serde_json::Value::Array(
                    vec.into_iter()
                        .map(|f| {
                            serde_json::Value::Number(
                                serde_json::Number::from_f64(f as f64).unwrap(),
                            )
                        })
                        .collect(),
                ))
            }
        } else {
            None
        };

        ChunkForUpload {
            id: chunk.id,
            vector,
            path: chunk.path,
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            file_hash: chunk.file_hash,
            chunk_hash: chunk.chunk_hash,
            file_mtime: chunk.file_mtime,
            file_ctime: chunk.file_ctime,
        }
    }
}

pub async fn write_chunks<S>(
    namespace: &str,
    chunks: S,
    delete_chunks: Option<Vec<Chunk>>,
) -> Result<(), TurbopufferError>
where
    S: Stream<Item = Chunk> + Send + 'static,
{
    const BATCH_SIZE: usize = 1000;
    const CONCURRENT_REQUESTS: usize = 4; // Reduced to prevent HTTP client exhaustion

    let api_key =
        std::env::var("TURBOPUFFER_API_KEY").map_err(|_| TurbopufferError::MissingApiKey)?;

    let namespace = namespace.to_string();
    let mut is_first_batch = true;
    let _total_start = Instant::now();
    let mut _total_written = 0;

    let mut chunk_stream = Box::pin(
        chunks
            .chunks(BATCH_SIZE)
            .map(move |batch| {
                let namespace = namespace.clone();
                let api_key = api_key.clone();
                let delete_chunks = if is_first_batch {
                    is_first_batch = false;
                    delete_chunks.clone()
                } else {
                    None
                };

                async move { write_batch(&namespace, batch, delete_chunks, &api_key).await }
            })
            .buffer_unordered(CONCURRENT_REQUESTS),
    );

    while let Some(result) = chunk_stream.next().await {
        let batch_count = result?;
        _total_written += batch_count;
    }

    Ok(())
}

async fn write_batch(
    namespace: &str,
    chunks: Vec<Chunk>,
    delete_chunks: Option<Vec<Chunk>>,
    api_key: &str,
) -> Result<usize, TurbopufferError> {
    let _instant = Instant::now();
    let chunk_count = chunks.len();
    let delete_count = delete_chunks.as_ref().map(|d| d.len()).unwrap_or(0);

    if chunk_count == 0 && delete_count == 0 {
        return Ok(0);
    }

    let client = get_client();

    let request_body = tokio_rayon::spawn(move || {
        let chunks_for_upload: Vec<ChunkForUpload> =
            chunks.into_iter().map(ChunkForUpload::from).collect();

        let mut request_body = serde_json::json!({
            "upsert_rows": chunks_for_upload,
            "distance_metric": "cosine_distance",
            "schema": {
                "file_hash": "uint",
                "chunk_hash": "uint"
            }
        });

        if let Some(delete_chunks) = delete_chunks {
            if !delete_chunks.is_empty() {
                let stale_paths: Vec<String> = delete_chunks
                    .into_iter()
                    .map(|c| c.path)
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();

                let filters: Vec<_> = stale_paths
                    .iter()
                    .map(|p| serde_json::json!(["path", "Eq", p]))
                    .collect();

                let delete_filter = if filters.len() == 1 {
                    filters[0].clone()
                } else {
                    serde_json::json!(["Or", filters])
                };

                request_body["delete_by_filter"] = delete_filter;
            }
        }
        request_body
    })
    .await;

    let response = client
        .post(format!(
            "https://{}.turbopuffer.com/v2/namespaces/{}",
            SETTINGS
                .get()
                .and_then(|s| s.turbopuffer_region.as_ref())
                .cloned()
                .unwrap_or_else(|| "gcp-us-east4".to_string()),
            namespace
        ))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request_body)
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(TurbopufferError::ApiError(error_text));
    }

    Ok(chunk_count)
}

pub async fn delete_namespace(namespace: &str) -> Result<(), TurbopufferError> {
    let api_key =
        std::env::var("TURBOPUFFER_API_KEY").map_err(|_| TurbopufferError::MissingApiKey)?;

    let client = get_client();

    let response = client
        .delete(format!(
            "https://{}.turbopuffer.com/v2/namespaces/{}",
            SETTINGS
                .get()
                .and_then(|s| s.turbopuffer_region.as_ref())
                .cloned()
                .unwrap_or_else(|| "gcp-us-east4".to_string()),
            namespace
        ))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(TurbopufferError::ApiError(error_text));
    }

    Ok(())
}

pub async fn query_chunks(
    namespace: &str,
    rank_by: serde_json::Value,
    top_k: u32,
    filters: Option<serde_json::Value>,
) -> Result<Vec<Chunk>, TurbopufferError> {
    let api_key =
        std::env::var("TURBOPUFFER_API_KEY").map_err(|_| TurbopufferError::MissingApiKey)?;

    let client = get_client();
    let _instant = Instant::now();

    let mut request = serde_json::json!({
        "rank_by": rank_by,
        "top_k": top_k,
        "exclude_attributes": ["vector"],
        "consistency": { "level": "eventual" },
    });

    if let Some(filters) = filters {
        request["filters"] = filters;
    }

    let response = client
        .post(format!(
            "https://{}.turbopuffer.com/v2/namespaces/{}/query",
            SETTINGS
                .get()
                .and_then(|s| s.turbopuffer_region.as_ref())
                .cloned()
                .unwrap_or_else(|| "gcp-us-east4".to_string()),
            namespace
        ))
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request)
        .send()
        .await?;

    if !response.status().is_success() {
        let error_text = response.text().await?;
        if error_text.contains("namespace") && error_text.contains("not found") {
            return Err(TurbopufferError::NamespaceNotFound(error_text));
        }
        return Err(TurbopufferError::ApiError(error_text));
    }

    let resp: QueryResponse = response.json().await?;

    Ok(resp.rows)
}

pub async fn all_chunks(namespace: &str) -> Result<Vec<Chunk>, TurbopufferError> {
    let _instant = Instant::now();
    let mut all_chunks = Vec::new();
    let mut last_id = 0u64;

    loop {
        let batch = query_chunks(
            namespace,
            serde_json::json!(["id", "asc"]),
            1200,
            if last_id > 0 {
                Some(serde_json::json!(["id", "Gt", last_id]))
            } else {
                None
            },
        )
        .await?;

        let batch_len = batch.len();
        if batch_len == 0 {
            break;
        }

        last_id = batch.last().unwrap().id;
        all_chunks.extend(batch);

        if batch_len < 1200 {
            break;
        }
    }

    Ok(all_chunks)
}

pub async fn all_server_chunks(namespace: &str) -> Result<Vec<Chunk>, TurbopufferError> {
    all_chunks(namespace).await
}
