use turbogrep::chunker;

#[test]
fn test_c_chunking() {
    let c_code = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// User structure to represent a user
typedef struct {
    char name[100];
    int age;
    char email[100];
} User;

// Creates a new user with the given parameters
User* create_user(const char* name, int age, const char* email) {
    User* user = (User*)malloc(sizeof(User));
    if (user == NULL) {
        return NULL;
    }
    
    strcpy(user->name, name);
    user->age = age;
    strcpy(user->email, email);
    
    return user;
}

// Returns the user's display name
void get_display_name(const User* user, char* buffer, size_t buffer_size) {
    snprintf(buffer, buffer_size, "%s (%d)", user->name, user->age);
}

// Validates the user's email format
int is_valid_email(const User* user) {
    return strchr(user->email, '@') != NULL && strchr(user->email, '.') != NULL;
}

// Calculates the factorial of a number
int factorial(int n) {
    if (n <= 1) {
        return 1;
    }
    return n * factorial(n - 1);
}

// Processes a list of users and returns statistics
void process_users(User** users, int count, int* minor_count, int* adult_count, int* senior_count) {
    *minor_count = 0;
    *adult_count = 0;
    *senior_count = 0;
    
    for (int i = 0; i < count; i++) {
        if (users[i]->age < 18) {
            (*minor_count)++;
        } else if (users[i]->age < 65) {
            (*adult_count)++;
        } else {
            (*senior_count)++;
        }
    }
}

// Calculates the average age of users
double calculate_average_age(User** users, int count) {
    if (count == 0) {
        return 0.0;
    }
    
    int total_age = 0;
    for (int i = 0; i < count; i++) {
        total_age += users[i]->age;
    }
    
    return (double)total_age / count;
}

// Frees memory allocated for a user
void free_user(User* user) {
    if (user != NULL) {
        free(user);
    }
}

int main() {
    // Example usage
    User* users[3];
    users[0] = create_user("Alice", 25, "alice@example.com");
    users[1] = create_user("Bob", 30, "bob@example.com");
    users[2] = create_user("Charlie", 17, "charlie@example.com");
    
    int minor_count, adult_count, senior_count;
    process_users(users, 3, &minor_count, &adult_count, &senior_count);
    
    printf("User statistics: minor=%d, adult=%d, senior=%d\n", 
           minor_count, adult_count, senior_count);
    
    // Clean up
    for (int i = 0; i < 3; i++) {
        free_user(users[i]);
    }
    
    return 0;
}
"#;

    // Create a temporary file with the C code
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.c");
    std::fs::write(&file_path, c_code).unwrap();

    // Test chunking
            let result = chunker::chunk_file(&file_path).unwrap();
        let chunks = result.chunks;

    // Verify we extracted functions
    assert!(!chunks.is_empty(), "Should extract at least one function");

    // Check that we have the expected functions
    let expected_functions = [
        "User* create_user(",
        "void get_display_name(",
        "int is_valid_email(",
        "int factorial(",
        "void process_users(",
        "double calculate_average_age(",
        "void free_user(",
        "int main(",
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
