use turbogrep::chunker;

#[test]
fn test_javascript_chunking() {
    let javascript_code = r#"
// User class to represent a user
class User {
    constructor(name, age, email) {
        this.name = name;
        this.age = age;
        this.email = email;
    }

    displayName() {
        return `${this.name} (${this.age})`;
    }

    isValidEmail() {
        return this.email.includes('@') && this.email.includes('.');
    }
}

// Calculates the factorial of a number
function factorial(n) {
    if (n <= 1) {
        return 1;
    }
    return n * factorial(n - 1);
}

// Processes a list of users and returns statistics
function processUsers(users) {
    const stats = {};
    
    for (const user of users) {
        let ageGroup;
        if (user.age < 18) {
            ageGroup = 'minor';
        } else if (user.age < 65) {
            ageGroup = 'adult';
        } else {
            ageGroup = 'senior';
        }
        
        stats[ageGroup] = (stats[ageGroup] || 0) + 1;
    }
    
    return stats;
}

// Calculates the average age of users
function calculateAverageAge(users) {
    if (users.length === 0) {
        return 0;
    }
    
    const totalAge = users.reduce((sum, user) => sum + user.age, 0);
    return totalAge / users.length;
}

// Arrow function example
const multiply = (a, b) => a * b;

// Function expression
const divide = function(a, b) {
    if (b === 0) {
        throw new Error('Division by zero');
    }
    return a / b;
};

// Example usage
const users = [
    new User('Alice', 25, 'alice@example.com'),
    new User('Bob', 30, 'bob@example.com'),
    new User('Charlie', 17, 'charlie@example.com'),
];

const stats = processUsers(users);
console.log('User statistics:', stats);
"#;

    // Create a temporary file with the JavaScript code
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.js");
    std::fs::write(&file_path, javascript_code).unwrap();

    // Test chunking
    let result = chunker::chunk_file(&file_path).unwrap();
    let chunks = result.chunks;

    // Verify we extracted functions
    assert!(!chunks.is_empty(), "Should extract at least one function");

    // Check that we have the expected functions
    let expected_functions = [
        "function factorial(",
        "function processUsers(",
        "function calculateAverageAge(",
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
