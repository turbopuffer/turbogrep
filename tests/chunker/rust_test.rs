use turbogrep::chunker;

#[test]
fn test_rust_chunking() {
    let rust_code = r#"
use std::collections::HashMap;

/// A simple struct to represent a user
pub struct User {
    pub name: String,
    pub age: u32,
    pub email: String,
}

impl User {
    /// Creates a new user with the given parameters
    pub fn new(name: String, age: u32, email: String) -> Self {
        Self {
            name,
            age,
            email,
        }
    }

    /// Returns the user's display name
    pub fn display_name(&self) -> String {
        format!("{} ({})", self.name, self.age)
    }

    /// Validates the user's email format
    pub fn is_valid_email(&self) -> bool {
        self.email.contains('@') && self.email.contains('.')
    }
}

/// Calculates the factorial of a number
fn factorial(n: u32) -> u32 {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}

/// Processes a list of users and returns statistics
fn process_users(users: Vec<User>) -> HashMap<String, u32> {
    let mut stats = HashMap::new();
    
    for user in users {
        let age_group = if user.age < 18 {
            "minor".to_string()
        } else if user.age < 65 {
            "adult".to_string()
        } else {
            "senior".to_string()
        };
        
        *stats.entry(age_group).or_insert(0) += 1;
    }
    
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_creation() {
        let user = User::new("Alice".to_string(), 25, "alice@example.com".to_string());
        assert_eq!(user.name, "Alice");
        assert_eq!(user.age, 25);
    }
}
"#;

    // Create a temporary file with the Rust code
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.rs");
    std::fs::write(&file_path, rust_code).unwrap();

    // Test chunking
            let result = chunker::chunk_file(&file_path).unwrap();
        let chunks = result.chunks;

    // Verify we extracted functions
    assert!(!chunks.is_empty(), "Should extract at least one function");

    // Check that we have the expected functions
    let expected_functions = [
        "fn new(",
        "fn display_name(",
        "fn is_valid_email(",
        "fn factorial(",
        "fn process_users(",
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

    // Check that we have the struct and impl blocks
    let expected_structs = [
        "pub struct User {",
        "impl User {",
    ];

    for expected_struct in expected_structs {
        let found = chunks.iter().any(|chunk| {
            chunk
                .content
                .as_ref()
                .map_or(false, |content| content.contains(expected_struct))
        });
        assert!(found, "Should have extracted struct/impl: {}", expected_struct);
    }

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
