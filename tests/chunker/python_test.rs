use turbogrep::chunker;

#[test]
fn test_python_chunking() {
    let python_code = r#"
import json
from typing import Dict, List, Optional

class User:
    """A simple class to represent a user."""
    
    def __init__(self, name: str, age: int, email: str):
        self.name = name
        self.age = age
        self.email = email
    
    def display_name(self) -> str:
        """Returns the user's display name."""
        return f"{self.name} ({self.age})"
    
    def is_valid_email(self) -> bool:
        """Validates the user's email format."""
        return '@' in self.email and '.' in self.email

def factorial(n: int) -> int:
    """Calculates the factorial of a number."""
    if n <= 1:
        return 1
    return n * factorial(n - 1)

def process_users(users: List[User]) -> Dict[str, int]:
    """Processes a list of users and returns statistics."""
    stats = {}
    
    for user in users:
        if user.age < 18:
            age_group = "minor"
        elif user.age < 65:
            age_group = "adult"
        else:
            age_group = "senior"
        
        stats[age_group] = stats.get(age_group, 0) + 1
    
    return stats

def calculate_average_age(users: List[User]) -> float:
    """Calculates the average age of users."""
    if not users:
        return 0.0
    
    total_age = sum(user.age for user in users)
    return total_age / len(users)

if __name__ == "__main__":
    # Example usage
    users = [
        User("Alice", 25, "alice@example.com"),
        User("Bob", 30, "bob@example.com"),
        User("Charlie", 17, "charlie@example.com"),
    ]
    
    stats = process_users(users)
    print(f"User statistics: {stats}")
"#;

    // Create a temporary file with the Python code
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.py");
    std::fs::write(&file_path, python_code).unwrap();

    // Test chunking
    let result = chunker::chunk_file(&file_path).unwrap();
    let chunks = result.chunks;

    // Verify we extracted functions
    assert!(!chunks.is_empty(), "Should extract at least one function");

    // Check that we have the expected functions
    let expected_functions = [
        "def __init__(",
        "def display_name(",
        "def is_valid_email(",
        "def factorial(",
        "def process_users(",
        "def calculate_average_age(",
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
