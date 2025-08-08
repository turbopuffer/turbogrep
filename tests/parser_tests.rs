use std::path::Path;
use turbogrep::chunker::{ChunkError, chunk};

const RUST_FIXTURE: &str = r#"
use std::collections::HashMap;

pub fn hello_world() {
    println!("Hello, World!");
}

fn calculate_sum(a: i32, b: i32) -> i32 {
    a + b
}

fn process_data<T>(data: T) -> T where T: Clone {
    data.clone()
}

async fn fetch_data() -> Result<String, Box<dyn std::error::Error>> {
    Ok("data".to_string())
}

struct Calculator;

impl Calculator {
    fn new() -> Self {
        Self
    }
    
    pub fn multiply(&self, x: i32, y: i32) -> i32 {
        x * y
    }
}

trait Processor {
    fn process(&self) -> String;
}

impl Processor for Calculator {
    fn process(&self) -> String {
        "processing".to_string()
    }
}

fn main() {
    hello_world();
}
"#;

#[test]
fn test_rust_function_extraction() {
    let path = Path::new("test_fixture.rs");

    // Create mock file metadata
    let metadata = std::fs::metadata("src/main.rs").unwrap(); // Use existing file for metadata

    let result = chunk(RUST_FIXTURE, path, metadata);

    assert!(
        result.is_ok(),
        "Failed to parse Rust fixture: {:?}",
        result.err()
    );

    let chunks = result.unwrap();

    // Should find some code chunks
    assert!(!chunks.is_empty(), "Should extract some chunks");

    // Check that we get chunks with function content
    let has_hello_world = chunks
        .iter()
        .any(|chunk| {
            chunk.content.as_ref()
                .map(|c| c.contains("hello_world") && c.contains("println!"))
                .unwrap_or(false)
        });
    assert!(has_hello_world, "Should contain hello_world function");

    println!("Extracted {} chunks", chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        println!(
            "Chunk {}: ({}:{}-{})",
            i + 1,
            chunk.path,
            chunk.start_line,
            chunk.end_line
        );
        println!(
            "  Content preview: {}",
            chunk.content.as_ref()
                .map(|c| c.chars().take(100).collect::<String>())
                .unwrap_or_else(|| "[no content]".to_string())
        );
    }
}

#[test]
fn test_unsupported_extension() {
    let path = Path::new("test_file.unknown");
    let metadata = std::fs::metadata("src/main.rs").unwrap(); // Use existing file for metadata
    let result = chunk("some content", path, metadata);

    assert!(
        result.is_err(),
        "Should return error for unsupported extension"
    );

    match result.unwrap_err() {
        ChunkError::UnsupportedExtension(ext) => {
            assert_eq!(ext, "unknown");
        }
        other => panic!("Expected UnsupportedExtension error, got: {:?}", other),
    }
}

#[test]
fn test_empty_file() {
    let path = Path::new("empty.rs");
    let metadata = std::fs::metadata("src/main.rs").unwrap(); // Use existing file for metadata
    let result = chunk("", path, metadata);

    assert!(
        result.is_ok(),
        "Should handle empty files: {:?}",
        result.err()
    );
    let chunks = result.unwrap();
    assert!(chunks.is_empty(), "Empty file should have no chunks");
}

#[test]
fn test_invalid_rust_syntax() {
    let path = Path::new("invalid.rs");
    let invalid_rust = "fn incomplete_function(";
    let metadata = std::fs::metadata("src/main.rs").unwrap(); // Use existing file for metadata
    let result = chunk(invalid_rust, path, metadata);

    match result {
        Ok(chunks) => {
            assert!(
                chunks.len() <= 1,
                "Invalid syntax should not produce many chunks"
            );
        }
        Err(_) => {
            // Error is also acceptable for invalid syntax
        }
    }
}

#[test]
fn test_ripgrep_supported_extensions() {
    let test_cases = vec![
        ("test.rs", true),
        ("test.py", true),
        ("test.pyi", true),
        ("test.js", true),
        ("test.jsx", true),
        ("test.mjs", true),
        ("test.ts", true),
        ("test.tsx", true),
        ("test.go", true),
        ("test.java", true),
        ("test.c", true),
        ("test.h", true),
        ("test.cpp", true),
        ("test.cc", true),
        ("test.cxx", true),
        ("test.hpp", true),
        ("test.unknown", false),
        ("test.txt", false),
        ("README.md", false),
    ];

    for (filename, should_be_supported) in test_cases {
        let path = Path::new(filename);
        let metadata = std::fs::metadata("src/main.rs").unwrap(); // Use existing file for metadata
        let result = chunk("// empty", path, metadata);

        if should_be_supported {
            match result {
                Ok(_) => {}
                Err(ChunkError::UnsupportedExtension(_)) => {
                    panic!(
                        "Expected {} to be supported by ripgrep's DEFAULT_TYPES",
                        filename
                    );
                }
                Err(_) => {}
            }
        } else {
            match result {
                Err(ChunkError::UnsupportedExtension(_)) => {}
                _ => panic!("Expected {} to be unsupported", filename),
            }
        }
    }
}
