use crate::config::SETTINGS;
use anyhow::Result;
use std::path::PathBuf;
use xxhash_rust::xxh3::xxh3_64;

/// Validate that a directory exists
pub fn validate_directory(path: &str) -> Result<PathBuf, String> {
    let path_buf = PathBuf::from(path);
    if !path_buf.exists() {
        Err(format!("Directory '{}' does not exist", path))
    } else if !path_buf.is_dir() {
        Err(format!("'{}' exists but is not a directory", path))
    } else {
        Ok(path_buf)
    }
}

pub fn find_project_root(start_path: &str) -> Result<std::path::PathBuf> {
    let mut current = std::path::Path::new(start_path).canonicalize()?;

    loop {
        // Check for project root indicators (ordered by priority)
        let indicators = [
            // Version control systems (highest priority)
            ".git",
            ".hg",
            ".svn",
            "_darcs",
            ".bzr",
            // Language-specific package managers & config files
            "Cargo.toml",    // Rust
            "package.json",  // Node.js/TypeScript/JavaScript
            "tsconfig.json", // TypeScript
            "deno.json",
            "deno.jsonc",       // Deno
            "pyproject.toml",   // Modern Python
            "setup.py",         // Python
            "requirements.txt", // pip
            "Pipfile",          // pipenv
            "poetry.lock",      // Poetry
            "environment.yml",  // Conda
            "go.mod",           // Go
            "Gemfile",          // Ruby
            "composer.json",    // PHP
            // Static site generators & documentation
            "mkdocs.yml",           // MkDocs
            "_config.yml",          // Jekyll
            "gatsby-config.js",     // Gatsby
            "next.config.js",       // Next.js
            "nuxt.config.js",       // Nuxt.js
            "docusaurus.config.js", // Docusaurus
            "hugo.toml",            // Hugo
            "hugo.yaml",            // Hugo
            // More language-specific files
            "stack.yaml",     // Haskell Stack
            "cabal.project",  // Haskell Cabal
            "Gemfile.lock",   // Ruby
            "yarn.lock",      // Yarn
            "pnpm-lock.yaml", // pnpm
            "bun.lockb",      // Bun
            "pubspec.yaml",   // Dart/Flutter
            "mix.exs",        // Elixir
            "rebar.config",   // Erlang
            "deps.edn",       // Clojure
            "project.clj",    // Leiningen
            "build.sbt",      // Scala
            "Package.swift",  // Swift
            "Podfile",        // iOS CocoaPods
            "Cartfile",       // iOS Carthage
            // Build systems
            "pom.xml",            // Maven (Java)
            "build.gradle",       // Gradle (Java/Android)
            "build.gradle.kts",   // Gradle Kotlin DSL
            "build.xml",          // Ant (Java)
            "CMakeLists.txt",     // CMake (C/C++)
            "Makefile",           // Make
            "meson.build",        // Meson
            "configure.ac",       // Autotools
            "configure.in",       // Autotools
            "Dockerfile",         // Docker
            "docker-compose.yml", // Docker Compose
            "Vagrantfile",        // Vagrant
            // IDE/Editor project files
            ".editorconfig", // Editor configuration
            ".vscode",       // VS Code workspace (directory)
            ".idea",         // IntelliJ IDEA (directory)
        ];

        for indicator in &indicators {
            if current.join(indicator).exists() {
                return Ok(current);
            }
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break, // Reached filesystem root
        }
    }

    // If no project root found, return the original canonicalized path
    Ok(std::path::Path::new(start_path).canonicalize()?)
}

pub fn namespace_and_dir(directory: &str) -> Result<(String, String)> {
    // Find the project root instead of using the provided directory directly
    let root_path = find_project_root(directory)?;

    // Get embedding provider from settings
    let embedding_provider = SETTINGS
        .get()
        .and_then(|s| s.embedding_provider.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("voyage");

    // Hash the root path for a consistent, short namespace name
    let path_str = root_path.to_string_lossy();
    let hash = xxh3_64(path_str.as_bytes());
    let namespace = format!("tg_{}_{:x}", embedding_provider, hash);

    // Return both namespace and the canonical root directory
    Ok((namespace, root_path.to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_validate_directory_exists() {
        // Test with current directory
        let current_dir = env::current_dir().unwrap();
        let result = validate_directory(&current_dir.to_string_lossy());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_directory_not_exists() {
        let result = validate_directory("/nonexistent/path/that/should/not/exist");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn test_find_project_root_current_dir() {
        // Should find the Cargo.toml in the current project
        let current_dir = env::current_dir().unwrap();
        let result = find_project_root(&current_dir.to_string_lossy());
        assert!(result.is_ok());

        let root = result.unwrap();
        assert!(root.join("Cargo.toml").exists());
    }

    #[test]
    fn test_namespace_and_dir_consistency() {
        // Test that the same directory always produces the same namespace
        let current_dir = env::current_dir().unwrap();
        let dir_str = current_dir.to_string_lossy();

        let result1 = namespace_and_dir(&dir_str);
        let result2 = namespace_and_dir(&dir_str);

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        let (ns1, dir1) = result1.unwrap();
        let (ns2, dir2) = result2.unwrap();

        assert_eq!(ns1, ns2);
        assert_eq!(dir1, dir2);
        assert!(ns1.starts_with("tg_"));
    }

    #[test]
    fn test_namespace_includes_embedding_provider() {
        // Test that the namespace includes the embedding provider
        let current_dir = env::current_dir().unwrap();
        let dir_str = current_dir.to_string_lossy();

        let result = namespace_and_dir(&dir_str);
        assert!(result.is_ok());
        
        let (namespace, _) = result.unwrap();
        // Namespace should include the embedding provider
        // Format: tg_{provider}_{hash}
        assert!(namespace.contains("_voyage_") || namespace.starts_with("tg_voyage_"));
    }
}
