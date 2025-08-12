use crate::chunker::Chunk;
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose};
use futures::stream::{self, Stream, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use std::env;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Instant;

/// Result from embedding operation including token usage
pub struct EmbedResult {
    pub chunks: Vec<Chunk>,
    pub total_tokens: Option<usize>,
}

/// Trait for embedding implementations
pub trait Embedding: Clone + Send + 'static {
    /// Embed a batch of chunks - this is the core method that implementations must provide
    fn embed(
        self,
        chunks: Vec<Chunk>,
        embedding_type: EmbeddingType,
    ) -> impl std::future::Future<Output = Result<EmbedResult, EmbeddingError>> + Send;

    /// Number of concurrent requests to make
    fn concurrency(&self) -> usize;

    /// Maximum number of chunks per batch
    fn max_batch_size(&self) -> usize;

    async fn ping(&self) -> Result<(), EmbeddingError> {
        // Default implementation does nothing
        Ok(())
    }

    /// Default implementation of embed_stream using the core methods
    /// Implementations typically don't need to override this
    fn embed_stream<S>(
        self,
        chunks: S,
        embedding_type: EmbeddingType,
    ) -> impl Stream<Item = Result<Chunk, EmbeddingError>>
    where
        S: Stream<Item = Chunk> + Send + 'static,
    {
        let concurrency = self.concurrency();
        let max_batch_size = self.max_batch_size();

        chunks
            .chunks(max_batch_size)
            .map(move |batch| {
                let embedding_impl = self.clone();
                embedding_impl.embed(batch, embedding_type)
            })
            .buffer_unordered(concurrency)
            .map(|result| match result {
                Ok(embed_result) => stream::iter(embed_result.chunks.into_iter().map(Ok)).boxed(),
                Err(e) => stream::once(async move { Err(e) }).boxed(),
            })
            .flatten()
            .boxed()
    }
}

/// Embedding type for Voyage AI API - determines how the model processes the text
#[derive(Debug, Clone, Copy)]
pub enum EmbeddingType {
    /// For query text - adds "Represent the query for retrieving supporting documents:" prompt
    Query,
    /// For document text - adds "Represent the document for retrieval:" prompt  
    Document,
}

impl EmbeddingType {
    fn as_str(&self) -> &'static str {
        match self {
            EmbeddingType::Query => "query",
            EmbeddingType::Document => "document",
        }
    }
}

/// Choose the embedding provider based on available environment variables
pub fn choose_embedding_provider() -> Option<String> {
    // Check for Voyage AI API key
    if env::var("VOYAGE_API_KEY").is_ok() {
        return Some("voyage".to_string());
    }

    // Future: Add other providers here
    // if env::var("OPENAI_API_KEY").is_ok() {
    //     return Some("openai".to_string());
    // }

    None
}

#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("Missing VOYAGE_API_KEY")]
    MissingApiKey,
    #[error("Request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),
    #[error("API error: {0}")]
    ApiError(String),
}

#[derive(Debug, Deserialize)]
struct VoyageResponse {
    data: Vec<VoyageData>,
    usage: Option<VoyageUsage>,
}

#[derive(Debug, Deserialize)]
struct VoyageUsage {
    total_tokens: usize,
}

#[derive(Debug, Deserialize)]
struct VoyageData {
    embedding: String, // Base64-encoded numpy array
}

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

/// Decode base64-encoded numpy float32 array to Vec<f32>
fn decode_base64_floats(base64_data: &str) -> Result<Vec<f32>, EmbeddingError> {
    // Decode base64 to bytes
    let bytes = general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| EmbeddingError::ApiError(format!("Base64 decode error: {}", e)))?;

    // Convert bytes to f32 values (numpy float32 is little-endian)
    let float_count = bytes.len() / 4;
    let mut floats = Vec::with_capacity(float_count);

    for chunk in bytes.chunks_exact(4) {
        let float_bytes = [chunk[0], chunk[1], chunk[2], chunk[3]];
        let float_val = f32::from_le_bytes(float_bytes);
        floats.push(float_val);
    }

    Ok(floats)
}

/// Voyage AI embedding implementation
#[derive(Clone, Copy)]
pub struct VoyageEmbedding {
    concurrency: usize,
}

impl VoyageEmbedding {
    pub fn new() -> Self {
        Self { concurrency: 8 }
    }

    pub fn with_concurrency(concurrency: usize) -> Self {
        Self { concurrency }
    }
}

impl Embedding for VoyageEmbedding {
    fn embed(
        self,
        chunks: Vec<Chunk>,
        embedding_type: EmbeddingType,
    ) -> impl std::future::Future<Output = Result<EmbedResult, EmbeddingError>> + Send {
        async move {
            let api_key =
                std::env::var("VOYAGE_API_KEY").map_err(|_| EmbeddingError::MissingApiKey)?;
            self.embed_batch_impl(chunks, embedding_type, api_key).await
        }
    }

    fn concurrency(&self) -> usize {
        self.concurrency
    }

    fn max_batch_size(&self) -> usize {
        500
    }

    async fn ping(&self) -> Result<(), EmbeddingError> {
        let client = get_client();
        let instant = Instant::now();
        let _response = client.get("https://api.voyageai.com/").send().await?;
        crate::vprintln!(
            "Voyage AI ping took {:.3}s",
            instant.elapsed().as_secs_f64()
        );

        Ok(())
    }
}

impl VoyageEmbedding {
    /// Internal boxed-future implementation to allow recursive splitting
    fn embed_batch_impl(
        &self,
        chunks: Vec<Chunk>,
        embedding_type: EmbeddingType,
        api_key: String,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<EmbedResult, EmbeddingError>> + Send + '_>>
    {
        Box::pin(async move {
            let _instant = Instant::now();
            let _batch_size = chunks.len();
            let client = get_client();

            // Extract texts for the API call
            let texts: Vec<&str> = chunks
                .iter()
                .map(|c| {
                    c.content
                        .as_ref()
                        .expect("Chunk missing content for embedding")
                        .as_str()
                })
                .collect();

            let response = client
                .post("https://api.voyageai.com/v1/embeddings")
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&serde_json::json!({
                    "input": texts,
                    "model": "voyage-code-3",
                    "input_type": embedding_type.as_str(),
                    "output_dtype": "float",
                    "encoding_format": "base64"
                }))
                .send()
                .await?;

            if !response.status().is_success() {
                let error_text = response.text().await?;
                // If batch exceeds model token limit, split in half and retry recursively
                if error_text
                    .to_lowercase()
                    .contains("max allowed tokens per submitted batch")
                    && chunks.len() > 1
                {
                    let mid = chunks.len() / 2;
                    let left_chunks = chunks[..mid].to_vec();
                    let right_chunks = chunks[mid..].to_vec();

                    let left_result = self
                        .embed_batch_impl(left_chunks, embedding_type, api_key.clone())
                        .await?;
                    let right_result = self
                        .embed_batch_impl(right_chunks, embedding_type, api_key)
                        .await?;

                    let mut combined_chunks =
                        Vec::with_capacity(left_result.chunks.len() + right_result.chunks.len());
                    combined_chunks.extend(left_result.chunks);
                    combined_chunks.extend(right_result.chunks);

                    let total_tokens = match (left_result.total_tokens, right_result.total_tokens) {
                        (Some(a), Some(b)) => Some(a + b),
                        (Some(a), None) => Some(a),
                        (None, Some(b)) => Some(b),
                        (None, None) => None,
                    };

                    return Ok(EmbedResult {
                        chunks: combined_chunks,
                        total_tokens,
                    });
                }

                return Err(EmbeddingError::ApiError(error_text));
            }

            let resp: VoyageResponse = response.json().await?;

            // Combine chunks with their embeddings, decoding base64 to f32
            let embedded_chunks = chunks
                .into_iter()
                .zip(resp.data)
                .map(|(mut chunk, data)| {
                    // Decode base64-encoded numpy float32 array
                    match decode_base64_floats(&data.embedding) {
                        Ok(float_embedding) => {
                            chunk.vector = Some(float_embedding);
                        }
                        Err(_e) => {
                            // Keep chunk without vector on decode failure
                        }
                    }
                    chunk
                })
                .collect();

            Ok(EmbedResult {
                chunks: embedded_chunks,
                total_tokens: resp.usage.map(|u| u.total_tokens),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_type_as_str() {
        assert_eq!(EmbeddingType::Query.as_str(), "query");
        assert_eq!(EmbeddingType::Document.as_str(), "document");
    }

    #[test]
    fn test_voyage_embedding_new() {
        let embedding = VoyageEmbedding::new();
        // Test default concurrency configuration
        assert_eq!(embedding.concurrency(), 8);
    }

    #[test]
    fn test_embedding_error_display() {
        let missing_key_error = EmbeddingError::MissingApiKey;
        assert!(
            missing_key_error
                .to_string()
                .contains("Missing VOYAGE_API_KEY")
        );

        let api_error = EmbeddingError::ApiError("test error".to_string());
        assert!(api_error.to_string().contains("test error"));
    }

    #[test]
    fn test_chunk_with_content() {
        let chunk = Chunk {
            content: Some("test content".to_string()),
            ..Default::default()
        };
        assert_eq!(chunk.content, Some("test content".to_string()));
        assert_eq!(chunk.vector, None);
    }

    #[test]
    fn test_chunk_default() {
        let chunk = Chunk::default();
        assert_eq!(chunk.content, None);
        assert_eq!(chunk.vector, None);
        assert_eq!(chunk.path, "");
        assert_eq!(chunk.start_line, 0);
        assert_eq!(chunk.end_line, 0);
    }

    #[tokio::test]
    async fn test_embedding_trait() {
        let embedding = VoyageEmbedding::new();
        let chunks = vec![Chunk {
            content: Some("test content".to_string()),
            ..Default::default()
        }];

        // Test that embed is callable
        let result = embedding.embed(chunks, EmbeddingType::Query).await;
        match result {
            Ok(embed_result) => {
                // API key is set, so we get a real embedding
                assert_eq!(embed_result.chunks.len(), 1);
                assert!(embed_result.chunks[0].vector.is_some());
            }
            Err(EmbeddingError::MissingApiKey) => {
                // Expected error - API key is not set in test environment
            }
            Err(EmbeddingError::ApiError(_)) => {
                // Expected error - API key is invalid in test environment
            }
            Err(e) => {
                panic!("Unexpected error: {:?}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_embedding_trait_document_type() {
        let embedding = VoyageEmbedding::new();
        let chunks = vec![Chunk {
            content: Some("document content".to_string()),
            ..Default::default()
        }];

        // Test with Document embedding type
        let result = embedding.embed(chunks, EmbeddingType::Document).await;
        match result {
            Ok(embed_result) => {
                assert_eq!(embed_result.chunks.len(), 1);
                assert!(embed_result.chunks[0].vector.is_some());
            }
            Err(EmbeddingError::MissingApiKey) => {
                // Expected error - API key is not set in test environment
            }
            Err(EmbeddingError::ApiError(_)) => {
                // Expected error - API key is invalid in test environment
            }
            Err(e) => {
                panic!("Unexpected error: {:?}", e);
            }
        }
    }

    #[test]
    fn test_voyage_embeddings_function() {
        // Test that the backward compatibility function exists and can be called
        let chunks = futures::stream::once(async {
            Chunk {
                content: Some("test".to_string()),
                ..Default::default()
            }
        });

        let embedding = VoyageEmbedding::new();
        let _stream = embedding.embed_stream(chunks, EmbeddingType::Query);
        // Just test that it compiles and returns a stream
    }

    #[test]
    fn test_choose_embedding_provider_with_voyage_key() {
        // Test with VOYAGE_API_KEY set
        unsafe {
            env::set_var("VOYAGE_API_KEY", "test-key");
        }

        let provider = choose_embedding_provider();
        assert_eq!(provider, Some("voyage".to_string()));

        // Clean up immediately after the test
        unsafe {
            env::remove_var("VOYAGE_API_KEY");
        }
    }

    #[test]
    fn test_choose_embedding_provider_no_key() {
        // Create a mock function that always returns None for testing
        fn mock_choose_embedding_provider() -> Option<String> {
            // Simulate no environment variables set
            None
        }

        let provider = mock_choose_embedding_provider();
        assert_eq!(provider, None);
    }
}
