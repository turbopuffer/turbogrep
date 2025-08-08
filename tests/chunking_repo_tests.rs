use std::path::Path;
use std::process::Command;
use turbogrep::chunker;

// Integration tests that clone real Git repositories and test chunking functionality
// This validates that our chunker works correctly on real-world codebases

#[tokio::test]
async fn test_chunking_ruby_on_rails() {
    let repo_path = setup_git_repo("https://github.com/rails/rails.git", "rails");

    if repo_path.is_none() {
        eprintln!("Skipping Rails test - git clone failed");
        return;
    }

    let repo_path = repo_path.unwrap();

    // Use chunk_files to process the entire repository (like turbogrep would)
    match chunker::chunk_files(&repo_path.to_string_lossy()) {
        Ok(all_chunks) => {
            println!("Found {} total chunks in Rails repo", all_chunks.len());

            // Filter to Ruby-related chunks for validation
            let ruby_chunks: Vec<_> = all_chunks
                .iter()
                .filter(|chunk| chunk.path.ends_with(".rb"))
                .collect();

            println!("Found {} Ruby chunks", ruby_chunks.len());

            // Basic validation - should have found some Ruby chunks
            assert!(
                ruby_chunks.len() > 100,
                "Should find many Ruby chunks in Rails"
            );

            // Validate a few sample chunks
            for chunk in ruby_chunks.iter().take(5) {
                assert!(!chunk.path.is_empty(), "Chunk path should not be empty");
                assert!(chunk.start_line > 0, "Start line should be positive");
                assert!(
                    chunk.end_line >= chunk.start_line,
                    "End line should be >= start line"
                );
                assert!(chunk.content.is_some(), "Chunk should have content");
                assert!(chunk.file_hash > 0, "File hash should be set");
                assert!(chunk.chunk_hash > 0, "Chunk hash should be set");
            }
        }
        Err(e) => {
            panic!("Failed to chunk Rails repository: {}", e);
        }
    }
}

#[tokio::test]
async fn test_chunking_cockroach_database() {
    let repo_path = setup_git_repo("https://github.com/cockroachdb/cockroach.git", "cockroach");

    if repo_path.is_none() {
        eprintln!("Skipping CockroachDB test - git clone failed");
        return;
    }

    let repo_path = repo_path.unwrap();

    // Use chunk_files to process the entire repository (like turbogrep would)
    match chunker::chunk_files(&repo_path.to_string_lossy()) {
        Ok(all_chunks) => {
            println!(
                "Found {} total chunks in CockroachDB repo",
                all_chunks.len()
            );

            // Filter to Go-related chunks for validation
            let go_chunks: Vec<_> = all_chunks
                .iter()
                .filter(|chunk| chunk.path.ends_with(".go"))
                .collect();

            println!("Found {} Go chunks", go_chunks.len());

            // Basic validation - should have found some Go chunks
            assert!(
                go_chunks.len() > 100,
                "Should find many Go chunks in CockroachDB"
            );

            // Validate a few sample chunks
            for chunk in go_chunks.iter().take(5) {
                assert!(!chunk.path.is_empty(), "Chunk path should not be empty");
                assert!(chunk.start_line > 0, "Start line should be positive");
                assert!(
                    chunk.end_line >= chunk.start_line,
                    "End line should be >= start line"
                );
                assert!(chunk.content.is_some(), "Chunk should have content");
                assert!(chunk.file_hash > 0, "File hash should be set");
                assert!(chunk.chunk_hash > 0, "Chunk hash should be set");
            }
        }
        Err(e) => {
            panic!("Failed to chunk CockroachDB repository: {}", e);
        }
    }
}

#[tokio::test]
async fn test_chunking_materialize_database() {
    let repo_path = setup_git_repo(
        "https://github.com/MaterializeInc/materialize.git",
        "materialize",
    );

    if repo_path.is_none() {
        eprintln!("Skipping Materialize test - git clone failed");
        return;
    }

    let repo_path = repo_path.unwrap();

    // Use chunk_files to process the entire repository (like turbogrep would)
    match chunker::chunk_files(&repo_path.to_string_lossy()) {
        Ok(all_chunks) => {
            println!(
                "Found {} total chunks in Materialize repo",
                all_chunks.len()
            );

            // Filter to Rust-related chunks for validation
            let rust_chunks: Vec<_> = all_chunks
                .iter()
                .filter(|chunk| chunk.path.ends_with(".rs"))
                .collect();

            println!("Found {} Rust chunks", rust_chunks.len());

            // Basic validation - should have found some Rust chunks
            assert!(
                rust_chunks.len() > 100,
                "Should find many Rust chunks in Materialize"
            );

            // Validate a few sample chunks
            for chunk in rust_chunks.iter().take(5) {
                assert!(!chunk.path.is_empty(), "Chunk path should not be empty");
                assert!(chunk.start_line > 0, "Start line should be positive");
                assert!(
                    chunk.end_line >= chunk.start_line,
                    "End line should be >= start line"
                );
                assert!(chunk.content.is_some(), "Chunk should have content");
                assert!(chunk.file_hash > 0, "File hash should be set");
                assert!(chunk.chunk_hash > 0, "Chunk hash should be set");
            }
        }
        Err(e) => {
            panic!("Failed to chunk Materialize repository: {}", e);
        }
    }
}

// Dramatically simplified git setup - just clone if not exists
fn setup_git_repo(url: &str, name: &str) -> Option<std::path::PathBuf> {
    let vendor_dir = Path::new("tests/vendor");
    let repo_path = vendor_dir.join(name);

    // If repo already exists, use it
    if repo_path.exists() {
        println!("Repository {} already exists, using cached version", name);
        return Some(repo_path);
    }

    // Create vendor directory if needed
    std::fs::create_dir_all(vendor_dir).ok()?;

    // Simple git clone
    println!("Cloning {} into {}...", url, repo_path.display());
    let output = Command::new("git")
        .arg("clone")
        .arg("--depth=1") // Shallow clone for speed
        .arg(url)
        .arg(&repo_path)
        .output()
        .ok()?;

    if output.status.success() {
        println!("Successfully cloned {}", name);
        Some(repo_path)
    } else {
        eprintln!(
            "Failed to clone {}: {}",
            name,
            String::from_utf8_lossy(&output.stderr)
        );
        None
    }
}
