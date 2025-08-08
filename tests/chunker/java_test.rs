use turbogrep::chunker;

#[test]
fn test_java_chunking() {
    let java_code = r#"
import java.util.HashMap;
import java.util.List;
import java.util.ArrayList;

/**
 * User class to represent a user in the system
 */
public class User {
    private String name;
    private int age;
    private String email;

    /**
     * Creates a new user with the given parameters
     */
    public User(String name, int age, String email) {
        this.name = name;
        this.age = age;
        this.email = email;
    }

    /**
     * Returns the user's display name
     */
    public String getDisplayName() {
        return name + " (" + age + ")";
    }

    /**
     * Validates the user's email format
     */
    public boolean isValidEmail() {
        return email.contains("@") && email.contains(".");
    }

    // Getters and setters
    public String getName() {
        return name;
    }

    public void setName(String name) {
        this.name = name;
    }

    public int getAge() {
        return age;
    }

    public void setAge(int age) {
        this.age = age;
    }

    public String getEmail() {
        return email;
    }

    public void setEmail(String email) {
        this.email = email;
    }
}

/**
 * Utility class for processing users
 */
public class UserProcessor {
    /**
     * Calculates the factorial of a number
     */
    public static int factorial(int n) {
        if (n <= 1) {
            return 1;
        }
        return n * factorial(n - 1);
    }

    /**
     * Processes a list of users and returns statistics
     */
    public static HashMap<String, Integer> processUsers(List<User> users) {
        HashMap<String, Integer> stats = new HashMap<>();
        
        for (User user : users) {
            String ageGroup;
            if (user.getAge() < 18) {
                ageGroup = "minor";
            } else if (user.getAge() < 65) {
                ageGroup = "adult";
            } else {
                ageGroup = "senior";
            }
            
            stats.put(ageGroup, stats.getOrDefault(ageGroup, 0) + 1);
        }
        
        return stats;
    }

    /**
     * Calculates the average age of users
     */
    public static double calculateAverageAge(List<User> users) {
        if (users.isEmpty()) {
            return 0.0;
        }
        
        int totalAge = 0;
        for (User user : users) {
            totalAge += user.getAge();
        }
        
        return (double) totalAge / users.size();
    }
}

/**
 * Main class with example usage
 */
public class Main {
    public static void main(String[] args) {
        List<User> users = new ArrayList<>();
        users.add(new User("Alice", 25, "alice@example.com"));
        users.add(new User("Bob", 30, "bob@example.com"));
        users.add(new User("Charlie", 17, "charlie@example.com"));
        
        HashMap<String, Integer> stats = UserProcessor.processUsers(users);
        System.out.println("User statistics: " + stats);
    }
}
"#;

    // Create a temporary file with the Java code
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("test.java");
    std::fs::write(&file_path, java_code).unwrap();

    // Test chunking
            let result = chunker::chunk_file(&file_path).unwrap();
        let chunks = result.chunks;

    // Verify we extracted functions
    assert!(!chunks.is_empty(), "Should extract at least one function");

    // Check that we have the expected functions
    let expected_functions = [
        "public String getDisplayName(",
        "public boolean isValidEmail(",
        "public String getName(",
        "public void setName(",
        "public static int factorial(",
        "public static HashMap<String, Integer> processUsers(",
        "public static double calculateAverageAge(",
        "public static void main(",
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
