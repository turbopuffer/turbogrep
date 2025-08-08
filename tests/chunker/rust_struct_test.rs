use turbogrep::chunker;

#[test]
fn test_rust_struct_chunking() {
    let rust_code = r#"
/// A simple struct with fields
struct Point {
    x: f64,
    y: f64,
}

/// A tuple struct
struct Color(u8, u8, u8);

/// A unit struct
struct Empty;

/// A generic struct
pub struct Container<T> {
    value: T,
}

/// An enum (not extracted as struct, but testing edge case)
enum Status {
    Active,
    Inactive,
}

impl Point {
    /// Creates a new point
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

impl<T> Container<T> {
    /// Creates a new container
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

/// A standalone function
fn calculate_distance(p1: &Point, p2: &Point) -> f64 {
    ((p2.x - p1.x).powi(2) + (p2.y - p1.y).powi(2)).sqrt()
}
"#;

    // Create a temporary file with the Rust code
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test_structs.rs");
    std::fs::write(&file_path, rust_code).unwrap();

    // Test chunking
    let result = chunker::chunk_file(&file_path).unwrap();
    let chunks = result.chunks;

    // Verify we extracted structs and functions
    assert!(!chunks.is_empty(), "Should extract at least one item");

    // Check that we have the expected structs
    let expected_structs = [
        "struct Point {",
        "struct Color(",
        "struct Empty;",
        "pub struct Container<T>",
    ];

    for expected_struct in expected_structs {
        let found = chunks.iter().any(|chunk| {
            chunk
                .content
                .as_ref()
                .map_or(false, |content| content.contains(expected_struct))
        });
        assert!(found, "Should have extracted struct: {}", expected_struct);
    }

    // Check that we have the impl blocks
    let expected_impls = [
        "impl Point {",
        "impl<T> Container<T> {",
    ];

    for expected_impl in expected_impls {
        let found = chunks.iter().any(|chunk| {
            chunk
                .content
                .as_ref()
                .map_or(false, |content| content.contains(expected_impl))
        });
        assert!(found, "Should have extracted impl: {}", expected_impl);
    }

    // Check that we have the functions
    let expected_functions = [
        "fn new(x: f64",
        "pub fn new(value: T)",
        "fn calculate_distance(",
    ];

    for expected_func in expected_functions {
        let found = chunks.iter().any(|chunk| {
            chunk
                .content
                .as_ref()
                .map_or(false, |content| content.contains(expected_func))
        });
        assert!(found, "Should have extracted function: {}", expected_func);
    }

    // Verify that doc comments are included with structs
    let point_chunk = chunks.iter().find(|chunk| {
        chunk
            .content
            .as_ref()
            .map_or(false, |content| content.contains("struct Point {"))
    });
    assert!(point_chunk.is_some(), "Should find Point struct chunk");
    assert!(
        point_chunk.unwrap()
            .content
            .as_ref()
            .unwrap()
            .contains("/// A simple struct with fields"),
        "Should include doc comment with struct"
    );

    // Verify chunk properties
    for chunk in &chunks {
        assert!(chunk.content.is_some(), "Chunk should have content");
        assert!(!chunk.path.is_empty(), "Chunk should have a path");
        assert!(chunk.start_line > 0, "Chunk should have start line");
        assert!(
            chunk.end_line >= chunk.start_line,
            "End line should be >= start line"
        );
    }
}