use turbogrep::chunker;

#[test]
fn test_cpp_chunking() {
    let cpp_code = r#"
#include <iostream>
#include <string>
#include <vector>
#include <map>

/**
 * User class to represent a user in the system
 */
class User {
private:
    std::string name;
    int age;
    std::string email;

public:
    /**
     * Creates a new user with the given parameters
     */
    User(const std::string& name, int age, const std::string& email)
        : name(name), age(age), email(email) {}

    /**
     * Returns the user's display name
     */
    std::string getDisplayName() const {
        return name + " (" + std::to_string(age) + ")";
    }

    /**
     * Validates the user's email format
     */
    bool isValidEmail() const {
        return email.find('@') != std::string::npos && 
               email.find('.') != std::string::npos;
    }

    // Getters
    std::string getName() const { return name; }
    int getAge() const { return age; }
    std::string getEmail() const { return email; }

    // Setters
    void setName(const std::string& name) { this->name = name; }
    void setAge(int age) { this->age = age; }
    void setEmail(const std::string& email) { this->email = email; }
};

/**
 * Utility class for processing users
 */
class UserProcessor {
public:
    /**
     * Calculates the factorial of a number
     */
    static int factorial(int n) {
        if (n <= 1) {
            return 1;
        }
        return n * factorial(n - 1);
    }

    /**
     * Processes a list of users and returns statistics
     */
    static std::map<std::string, int> processUsers(const std::vector<User>& users) {
        std::map<std::string, int> stats;
        
        for (const auto& user : users) {
            std::string ageGroup;
            if (user.getAge() < 18) {
                ageGroup = "minor";
            } else if (user.getAge() < 65) {
                ageGroup = "adult";
            } else {
                ageGroup = "senior";
            }
            
            stats[ageGroup]++;
        }
        
        return stats;
    }

    /**
     * Calculates the average age of users
     */
    static double calculateAverageAge(const std::vector<User>& users) {
        if (users.empty()) {
            return 0.0;
        }
        
        int totalAge = 0;
        for (const auto& user : users) {
            totalAge += user.getAge();
        }
        
        return static_cast<double>(totalAge) / users.size();
    }
};

/**
 * Main function with example usage
 */
int main() {
    std::vector<User> users;
    users.emplace_back("Alice", 25, "alice@example.com");
    users.emplace_back("Bob", 30, "bob@example.com");
    users.emplace_back("Charlie", 17, "charlie@example.com");
    
    auto stats = UserProcessor::processUsers(users);
    std::cout << "User statistics: ";
    for (const auto& [group, count] : stats) {
        std::cout << group << "=" << count << " ";
    }
    std::cout << std::endl;
    
    return 0;
}
"#;

    // Create a temporary file with the C++ code
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.cpp");
    std::fs::write(&file_path, cpp_code).unwrap();

    // Test chunking
            let result = chunker::chunk_file(&file_path).unwrap();
        let chunks = result.chunks;

    // Verify we extracted functions
    assert!(!chunks.is_empty(), "Should extract at least one function");

    // Check that we have the expected functions
    let expected_functions = [
        "User(const std::string& name, int age, const std::string& email)",
        "std::string getDisplayName() const",
        "bool isValidEmail() const",
        "std::string getName() const",
        "void setName(const std::string& name)",
        "static int factorial(int n)",
        "static std::map<std::string, int> processUsers(",
        "static double calculateAverageAge(",
        "int main()",
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
