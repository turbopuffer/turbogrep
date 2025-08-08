# Turbogrep

A fast function extraction, embedding, and vector storage tool using TreeSitter, Voyage AI, and turbopuffer.

## Features

### Function Extraction
- Extract complete function source code from multiple programming languages
- Uses TreeSitter for accurate parsing with line numbers and metadata
- Supports Rust, Python, JavaScript, TypeScript, Go, Java, C, and C++
- Leverages ripgrep's file type detection for comprehensive language support

### Embeddings API
- Generate embeddings using Voyage AI's voyage-3.5 model
- Async HTTP client with lazy global initialization
- Simple function-based API

### Vector Storage & Search
- Store function embeddings in turbopuffer with rich metadata
- Query for similar functions using vector search
- Includes file path, line numbers, function names, and full content

## Usage

### Syncing Code (Index, Embed, and Sync with Turbopuffer)

```bash
# 1. Set your API keys
export VOYAGE_API_KEY="your_voyage_api_key_here"
export TURBOPUFFER_API_KEY="your_turbopuffer_api_key_here"

# 2. Sync your codebase (extract, embed, and upload to Turbopuffer)
cargo run --release -- sync ~/path/to/your/codebase
```

**What this does:**
- Extracts all functions/methods from your codebase
- Generates embeddings for each chunk
- Uploads new/changed chunks to Turbopuffer, deletes stale ones

**Sample output:**
```
Syncing codebase at ~/src/myproject
Found 128 code chunks
Uploading 12 new/changed chunks
Deleting 3 stale chunks from Turbopuffer
Sync complete!
```

### Code Chunking (Extract Functions/Methods Only)

```bash
# Extract and print all code chunks (functions/methods) in a directory
cargo run --release -- chunk ~/path/to/your/codebase
```

**Sample output:**
```
src/main.rs:12-34 fn main() { ... }
src/lib.rs:5-20 fn process_data(input: &str) -> Result<()> { ... }
src/utils.py:10-18 def parse_config(path): ...
...
```

### Direct Rust API Example: Code Chunking

```rust
use turbogrep::chunker::{chunk_files, Chunk};

fn main() {
    let chunks: Vec<Chunk> = chunk_files("./src").unwrap();
    for chunk in chunks {
        println!(
            "{}:{}-{} {}",
            chunk.path,
            chunk.start_line,
            chunk.end_line,
            chunk.content.as_deref().unwrap_or("<no content>")
        );
    }
}
```

### Embeddings API (Rust)

```rust
use futures::StreamExt;
use turbogrep::{chunker::Chunk, embeddings::{voyage_embeddings, EmbeddingType, EmbeddingError}};

#[tokio::main]
async fn main() -> Result<(), EmbeddingError> {
    let chunks: Vec<Chunk> = vec![
        Chunk { content: Some("Hello, world!".to_string()), ..Default::default() },
        Chunk { content: Some("This is a sample text for embedding.".to_string()), ..Default::default() },
    ];
    
    let mut stream = VoyageEmbedding::new().embed_stream(futures::stream::iter(chunks), EmbeddingType::Document);
    let mut count = 0;
    while let Some(res) = stream.next().await {
        let _embedded = res?;
        count += 1;
    }
    println!("Created {} embeddings", count);
    Ok(())
}
```

### Turbopuffer Vector Search (Rust)

```rust
use futures::StreamExt;
use turbogrep::{chunker::Chunk, embeddings::{voyage_embeddings, EmbeddingType}, turbopuffer::query_chunks};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let query_text = "function that prints hello world";
    
    // Create embedding for the query
    let query_chunk = Chunk {
        content: Some(query_text.to_string()),
        ..Default::default()
    };
    let mut stream = VoyageEmbedding::new().embed_stream(futures::stream::iter(vec![query_chunk]), EmbeddingType::Query);
    let query_vector = stream.next().await.ok_or("No embedding returned for query")??.vector.unwrap_or_default();
    
    // Query turbopuffer for similar functions
    let results = query_chunks(
        "turbogrep", 
        serde_json::json!(["vector", "ANN", query_vector]), 
        5, 
        None
    ).await?;
    
    for result in results {
        println!(
            "{} (lines {}-{})",
            result.path,
            result.start_line,
            result.end_line,
        );
    }
    Ok(())
}
```

## Implementation Details

### Concurrent Architecture

The indexing process optimizes performance by running independent operations concurrently:

1. **Parallel Operations**: `chunk_files()` (local parsing) and `all_server_chunks()` (remote fetch) run simultaneously
2. **Diff Calculation**: `tpuf_chunk_diff()` compares local and remote chunks to determine what needs updating
3. **Stream Processing**: `tpuf_apply_diff()` applies changes using streaming APIs with internal batching and concurrency

### Key Components

- **Chunking**: Uses tree-sitter to parse code into semantic chunks (functions/methods)
- **Embeddings**: Voyage AI generates embeddings with internal batching (100 chunks) and concurrency (3 requests)
- **Storage**: Turbopuffer handles vector storage with batched writes (1000 chunks) and concurrent requests
- **Deduplication**: xxHash-based content hashing prevents redundant processing

## Dependencies

- `tree-sitter` - For syntax-aware parsing
- `ignore` - For file traversal using ripgrep's file type detection
- `reqwest` - For HTTP requests to APIs
- `serde` - For JSON serialization/deserialization
- `tokio` - For async runtime
- `globset` - For glob pattern matching

## License

MIT 