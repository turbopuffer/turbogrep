use turbogrep::chunker;

#[test]
fn test_go_chunking() {
    let go_code = r#"
package main

import (
	"fmt"
	"strings"
)

// User represents a user in the system
type User struct {
	Name  string
	Age   int
	Email string
}

// NewUser creates a new user with the given parameters
func NewUser(name string, age int, email string) *User {
	return &User{
		Name:  name,
		Age:   age,
		Email: email,
	}
}

// DisplayName returns the user's display name
func (u *User) DisplayName() string {
	return fmt.Sprintf("%s (%d)", u.Name, u.Age)
}

// IsValidEmail validates the user's email format
func (u *User) IsValidEmail() bool {
	return strings.Contains(u.Email, "@") && strings.Contains(u.Email, ".")
}

// Factorial calculates the factorial of a number
func Factorial(n int) int {
	if n <= 1 {
		return 1
	}
	return n * Factorial(n-1)
}

// ProcessUsers processes a list of users and returns statistics
func ProcessUsers(users []*User) map[string]int {
	stats := make(map[string]int)
	
	for _, user := range users {
		var ageGroup string
		if user.Age < 18 {
			ageGroup = "minor"
		} else if user.Age < 65 {
			ageGroup = "adult"
		} else {
			ageGroup = "senior"
		}
		
		stats[ageGroup]++
	}
	
	return stats
}

// CalculateAverageAge calculates the average age of users
func CalculateAverageAge(users []*User) float64 {
	if len(users) == 0 {
		return 0.0
	}
	
	totalAge := 0
	for _, user := range users {
		totalAge += user.Age
	}
	
	return float64(totalAge) / float64(len(users))
}

func main() {
	// Example usage
	users := []*User{
		NewUser("Alice", 25, "alice@example.com"),
		NewUser("Bob", 30, "bob@example.com"),
		NewUser("Charlie", 17, "charlie@example.com"),
	}
	
	stats := ProcessUsers(users)
	fmt.Printf("User statistics: %v\n", stats)
}
"#;

    // Create a temporary file with the Go code
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.go");
    std::fs::write(&file_path, go_code).unwrap();

    // Test chunking
            let result = chunker::chunk_file(&file_path).unwrap();
        let chunks = result.chunks;

    // Verify we extracted functions
    assert!(!chunks.is_empty(), "Should extract at least one function");

    // Check that we have the expected functions
    let expected_functions = [
        "func NewUser(",
        "func (u *User) DisplayName(",
        "func (u *User) IsValidEmail(",
        "func Factorial(",
        "func ProcessUsers(",
        "func CalculateAverageAge(",
        "func main(",
    ];

    for expected_func in expected_functions {
        let found = chunks.iter().any(|chunk| {
            chunk.content.as_ref()
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
