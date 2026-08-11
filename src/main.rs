use crate::embeddings::Embedding;
use anyhow::Result;
use clap::Parser;
use owo_colors::OwoColorize;
use rand::prelude::*;
use rand::rngs::StdRng;
use std::path::Path;
use turbogrep::{config, is_verbose, namespace_and_dir, vprintln};

mod chunker;
mod embeddings;
mod progress;
mod project;
mod scip;
mod search;
mod sync;
mod turbopuffer;

/// Parse CLI arguments with ripgrep-style logic
fn parse_cli_args(cli: &Cli) -> Result<(Option<String>, String), String> {
    let (query, start_directory) = match (&cli.pattern, &cli.path) {
        (None, None) => {
            // No arguments - index current directory
            (
                None,
                std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            )
        }
        (Some(pattern), None) => {
            // Single argument - check if it's a directory or a query
            if Path::new(pattern).is_dir() {
                // turbogrep PATH - index directory only
                (None, pattern.clone())
            } else if Path::new(pattern).exists() {
                // Path exists but is not a directory - this is an error
                return Err(format!(r#"'{pattern}' exists but is not a directory"#));
            } else if pattern.starts_with('/') || pattern.starts_with('.') {
                // Argument looks like a path but doesn't exist - warn user
                eprintln!(
                    "<(°~°)> Warning: '{pattern}' looks like a directory path but doesn't exist.",
                );
                eprintln!(
                    "<(°◯°)> Treating '{pattern}' as a search query and searching current directory.",
                );
                eprintln!("<(°◯°)> If you meant to specify a directory, please check the path.");
                let directory = std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                (Some(pattern.clone()), directory)
            } else {
                // turbogrep PATTERN - search current directory
                let directory = std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                (Some(pattern.clone()), directory)
            }
        }
        (Some(pattern), Some(path)) => {
            // turbogrep PATTERN PATH - validate directory exists
            project::validate_directory(path)?;
            (Some(pattern.clone()), path.clone())
        }
        (None, Some(path)) => {
            // turbogrep PATH - index directory only
            project::validate_directory(path)?;
            (None, path.clone())
        }
    };

    Ok((query, start_directory))
}

/// Sample N random chunks with deterministic seeding based on directory path
fn sample_random_chunks(
    chunks: Vec<chunker::Chunk>,
    n: usize,
    seed_data: &str,
) -> Vec<chunker::Chunk> {
    if chunks.len() <= n {
        return chunks;
    }

    // Create deterministic seed from directory path
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    seed_data.hash(&mut hasher);
    let seed = hasher.finish();

    let mut rng = StdRng::seed_from_u64(seed);
    let mut sampled: Vec<_> = chunks.into_iter().collect();

    // Sort deterministically first to ensure consistent input for shuffling
    sampled.sort_by(|a, b| a.path.cmp(&b.path).then(a.start_line.cmp(&b.start_line)));

    sampled.shuffle(&mut rng);
    sampled.truncate(n);
    sampled
}

/// Fast semantic code search powered by AI embeddings and turbopuffer
#[derive(Parser)]
#[command(name = "tg")]
#[command(version = "0.1.0")]
#[command(about = "Fast semantic code search powered by AI embeddings and turbopuffer")]
#[command(long_about = "
turbogrep uses AI embeddings via Voyage AI to enable semantic code search.
It stores vectors in turbopuffer for fast similarity search across codebases.

EXAMPLES:
    tg \"async function\"                     Search current directory  
    tg \"error handling\" ./src               Search specific directory
    tg ./src                               Index directory only
    tg --reset .                           Reset index and sync
    tg --no-sync \"query\" .                  Search without syncing

REGIONS:
    Common turbopuffer regions: gcp-us-central1, gcp-us-east1, gcp-us-west1,
    gcp-europe-west1, gcp-europe-west4, gcp-asia-southeast1

ENVIRONMENT:
    TURBOPUFFER_API_KEY                     Required for vector storage
    VOYAGE_API_KEY                          Required for AI embeddings
")]
struct Cli {
    /// Search query (semantic search using AI embeddings)
    #[arg(value_name = "PATTERN")]
    pattern: Option<String>,

    /// Directory to search/index (default: current directory)
    #[arg(value_name = "PATH")]
    path: Option<String>,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Only chunk files (no embedding/indexing)
    #[arg(long)]
    chunk_only: bool,

    /// Delete namespace and perform fresh sync
    #[arg(long)]
    reset: bool,

    /// Search existing index only (skip sync)
    #[arg(long)]
    no_sync: bool,

    /// Index/sync only, don't search (even if query provided)
    #[arg(long)]
    no_search: bool,

    /// Maximum number of results to return
    #[arg(short = 'm', long = "max-count", default_value = "20")]
    max_count: usize,

    /// Output N random (seeded) chunks to stdout
    #[arg(long = "sample")]
    sample: Option<usize>,

    /// Override embedding provider concurrency (default: 2)
    /// Higher values = faster embedding but more API load
    #[arg(long = "embedding-concurrency")]
    embedding_concurrency: Option<usize>,

    /// Show distance scores in output (lower is better)
    #[arg(long)]
    scores: bool,

    /// Use turbopuffer native embeddings (skips Voyage API entirely)
    #[arg(long)]
    embeddings: bool,

    /// Search index using a regex pattern instead of semantic search
    #[arg(long)]
    regex: bool,

    /// Embedding model name (default: voyage/voyage-code-3)
    #[arg(long, default_value = "voyage/voyage-code-3")]
    model: String,

    /// turbopuffer region to use (overrides config file)
    #[arg(long)]
    region: Option<String>,

    /// Custom namespace name (overrides auto-derived name from directory path)
    #[arg(long)]
    name: Option<String>,

    /// Print each chunk as it is indexed (path, line range, and content)
    #[arg(long)]
    debug_chunks: bool,

    /// Add chunks to namespace without deleting stale ones (for multi-repo namespaces)
    #[arg(long)]
    multi: bool,

    /// Generate a SCIP index for the target TypeScript project and exit
    #[arg(long)]
    scip: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    turbogrep::set_verbose(cli.verbose);

    if let Err(e) = config::load_or_init_settings(cli.region.as_deref()).await {
        eprintln!("<(°!°)> Error loading settings: {e}");
        return;
    }

    // Parse clap arguments with ripgrep-style logic
    let (query, start_directory) = match parse_cli_args(&cli) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("<(°!°)> Error: {e}");
            return;
        }
    };

    // Generate a SCIP index, enrich the matching function chunks with definitions/references
    // sparse vectors, and upsert them into the same namespace as regular indexing, then exit.
    if cli.scip {
        let (derived_namespace, root_dir) = namespace_and_dir(&start_directory).unwrap();
        let namespace = cli.name.clone().unwrap_or(derived_namespace);

        vprintln!(
            "<(°○°)> Note: --scip upserts sparse-only rows and will clear the dense vector \
             on any chunk that was already embedded by a regular `tg` run, until the next sync."
        );

        let scip::ScipIndexResult { chunks, unmatched } = match scip::index(Path::new(&root_dir)) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("<(°!°)> Failed to build SCIP index: {e}");
                std::process::exit(1);
            }
        };

        if unmatched > 0 {
            println!(
                "<(°◯°)> {} function chunk{} could not be matched to a SCIP symbol (rerun with -v for details)",
                unmatched,
                if unmatched == 1 { "" } else { "s" }
            );
        }

        if chunks.is_empty() {
            println!("<(°◯°)> No function chunks could be matched to SCIP symbols");
            return;
        }

        if cli.debug_chunks {
            for chunk in &chunks {
                eprintln!("{}:{}-{}", chunk.path, chunk.start_line, chunk.end_line);
                if let Some(definitions) = &chunk.definitions {
                    eprintln!("  definitions: {:?}", definitions);
                }
                if let Some(references) = &chunk.references {
                    eprintln!("  references:  {:?}", references);
                }
            }
        }

        let chunk_count = chunks.len();
        if let Err(e) =
            turbopuffer::write_chunks(&namespace, futures::stream::iter(chunks), None, None).await
        {
            eprintln!("<(°!°)> Failed to write SCIP chunks: {e}");
            std::process::exit(1);
        }

        println!(
            "<(°◕°)> Wrote {} SCIP-enriched chunks to namespace {}",
            chunk_count, namespace
        );
        return;
    }

    // If reset flag is provided, delete the namespace first
    if cli.reset {
        let namespace = if let Some(ref name) = cli.name {
            name.clone()
        } else {
            namespace_and_dir(&start_directory).unwrap().0
        };
        vprintln!("<(°○°)> Resetting namespace: {}", namespace);
        if let Err(e) = turbopuffer::delete_namespace(&namespace).await {
            vprintln!("<(°◯°)> Note: {}", e);
        }
        sync::tpuf_sync(&start_directory, cli.embedding_concurrency, cli.embeddings, &cli.model, cli.name.as_deref(), cli.debug_chunks, cli.multi)
            .await
            .unwrap();
    }

    // Handle --sample flag: output N random chunks to stdout
    if let Some(sample_count) = cli.sample {
        let (_, root_dir) = namespace_and_dir(&start_directory).unwrap();
        let chunks = chunker::chunk_files(&root_dir).unwrap();
        let sampled_chunks = sample_random_chunks(chunks, sample_count, &start_directory);

        for chunk in sampled_chunks {
            if let Some(content) = &chunk.content {
                println!(
                    "{}",
                    format!(
                        "{path}:{start_line}:{end_line}",
                        path = chunk.path,
                        start_line = chunk.start_line,
                        end_line = chunk.end_line
                    )
                    .bright_cyan()
                );
                println!("{}", content);
                println!(); // Empty line separator
            }
        }
        return;
    }

    if cli.chunk_only {
        // Only run the chunking step for performance testing
        let (_, root_dir) = namespace_and_dir(&start_directory).unwrap();
        let chunks = chunker::chunk_files(&root_dir).unwrap();
        if cli.debug_chunks {
            for chunk in &chunks {
                eprintln!(
                    "{}:{}-{}",
                    chunk.path, chunk.start_line, chunk.end_line
                );
                if let Some(content) = &chunk.content {
                    eprintln!("{}", content);
                }
                eprintln!();
            }
        };
    } else if query.is_none() || cli.no_search {
        // No query provided, just sync the directory
        vprintln!(
            "No search query provided, syncing directory: {}",
            start_directory
        );
        sync::tpuf_sync(&start_directory, cli.embedding_concurrency, cli.embeddings, &cli.model, cli.name.as_deref(), cli.debug_chunks, cli.multi)
            .await
            .unwrap();
    } else if let Some(query) = query {
        // Warm up turbopuffer connections in the background to reduce first-call latency
        tokio::spawn(async {
            for _i in 1..=5 {
                if let Err(_e) = turbopuffer::ping(None).await {
                    break;
                }
            }
        });

        if !cli.embeddings {
            tokio::spawn(async {
                let voyage = embeddings::VoyageEmbedding::new();
                for _i in 1..=5 {
                    if let Err(_e) = voyage.ping().await {
                        break;
                    }
                }
            });
        }

        if cli.regex {
            if !cli.no_sync && !cli.reset {
                if let Err(e) = sync::tpuf_sync(&start_directory, cli.embedding_concurrency, cli.embeddings, &cli.model, cli.name.as_deref(), cli.debug_chunks, cli.multi).await {
                    eprintln!("<(°!°)> Sync failed: {e}");
                    std::process::exit(1);
                }
            }
            match search::search_regex(&query, &start_directory, cli.max_count, cli.scores, cli.name.as_deref()).await {
                Ok(results) => println!("{results}"),
                Err(e) => {
                    eprintln!("<(°!°)> Regex search failed: {e}");
                    std::process::exit(1);
                }
            }
        } else if cli.reset {
            // no need to speculate, we know it's indexed
            match search::search(
                &query,
                &start_directory,
                cli.max_count,
                cli.embedding_concurrency,
                cli.scores,
                cli.embeddings,
                &cli.model,
                cli.name.as_deref(),
            )
            .await
            {
                Ok(results) => println!("{results}"),
                Err(e) => {
                    eprintln!("<(°!°)> Search failed: {e}");
                    std::process::exit(1);
                }
            }
        } else if cli.no_sync {
            vprintln!("<(°◯°)> Searching existing index (--no-sync)...");
            match search::search(
                &query,
                &start_directory,
                cli.max_count,
                cli.embedding_concurrency,
                cli.scores,
                cli.embeddings,
                &cli.model,
                cli.name.as_deref(),
            )
            .await
            {
                Ok(results) => println!("{results}"),
                Err(e) => {
                    eprintln!("<(°!°)> Search failed: {e}");
                    std::process::exit(1);
                }
            }
        } else {
            match search::speculate_search(
                &query,
                &start_directory,
                cli.max_count,
                cli.embedding_concurrency,
                cli.scores,
                cli.embeddings,
                &cli.model,
                cli.name.as_deref(),
                cli.multi,
            )
            .await
            {
                Ok(results) => println!("{results}"),
                Err(e) => {
                    eprintln!("<(°!°)> Search failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    } else {
        unreachable!("This should never happen - query should always be Some or None");
    }
}
