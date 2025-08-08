use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Settings {
    pub turbopuffer_region: Option<String>,
    pub embedding_provider: Option<String>,
}

pub static SETTINGS: OnceLock<Settings> = OnceLock::new();

fn config_path() -> Result<PathBuf> {
    let config_dir = get_config_dir()?;
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }
    Ok(config_dir.join("config.json"))
}

fn get_config_dir() -> Result<PathBuf> {
    if cfg!(target_os = "windows") {
        // Windows: %APPDATA%\turbogrep
        let appdata = std::env::var("APPDATA").context("APPDATA environment variable not set")?;
        Ok(PathBuf::from(appdata).join("turbogrep"))
    } else {
        // Unix/Linux/macOS: Use XDG Base Directory Specification
        if let Ok(xdg_config_home) = std::env::var("XDG_CONFIG_HOME") {
            Ok(PathBuf::from(xdg_config_home).join("turbogrep"))
        } else {
            let home = std::env::var("HOME").context("HOME environment variable not set")?;
            Ok(PathBuf::from(home).join(".config/turbogrep"))
        }
    }
}

pub async fn load_or_init_settings() -> Result<()> {
    let path = config_path()?;
    let mut settings = if path.exists() {
        let content = fs::read_to_string(&path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        Settings::default()
    };


    let mut config_changed = false;

    if settings.turbopuffer_region.is_none() {
        match crate::turbopuffer::find_closest_region().await {
            Ok(best_region) => {
                settings.turbopuffer_region = Some(best_region);
                config_changed = true;
            }
            Err(_e) => {
                settings.turbopuffer_region = Some("gcp-us-east4".to_string());
                config_changed = true;
            }
        }
    }

    if settings.embedding_provider.is_none() {
        settings.embedding_provider = crate::embeddings::choose_embedding_provider();
        config_changed = true;
    }

    if config_changed {
        let content = serde_json::to_string_pretty(&settings)?;
        fs::write(&path, content)?;
    }

    SETTINGS
        .set(settings)
        .map_err(|_| anyhow::anyhow!("Failed to set SETTINGS"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_settings_default() {
        let settings = Settings::default();
        assert_eq!(settings.turbopuffer_region, None);
        assert_eq!(settings.embedding_provider, None);
    }

    #[test]
    fn test_settings_serialization() {
        let settings = Settings {
            turbopuffer_region: Some("test-region".to_string()),
            embedding_provider: Some("voyage".to_string()),
        };

        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(
            deserialized.turbopuffer_region,
            Some("test-region".to_string())
        );
        assert_eq!(deserialized.embedding_provider, Some("voyage".to_string()));
    }

    #[test]
    fn test_settings_with_none_region() {
        let settings = Settings {
            turbopuffer_region: None,
            embedding_provider: None,
        };

        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.turbopuffer_region, None);
        assert_eq!(deserialized.embedding_provider, None);
    }

    #[test]
    fn test_config_path_unix() {
        if cfg!(unix) {
            let original_home = env::var("HOME");
            let original_xdg = env::var("XDG_CONFIG_HOME");

            // Test with XDG_CONFIG_HOME set
            unsafe {
                env::set_var("XDG_CONFIG_HOME", "/tmp/test_config");
                env::set_var("HOME", "/tmp/test_home");
            }

            let result = get_config_dir();
            assert!(result.is_ok());
            let path = result.unwrap();
            assert_eq!(path, PathBuf::from("/tmp/test_config/turbogrep"));

            // Test with XDG_CONFIG_HOME unset
            unsafe {
                env::remove_var("XDG_CONFIG_HOME");
            }
            let result = get_config_dir();
            assert!(result.is_ok());
            let path = result.unwrap();
            assert_eq!(path, PathBuf::from("/tmp/test_home/.config/turbogrep"));

            // Restore original environment
            unsafe {
                if let Ok(original) = original_home {
                    env::set_var("HOME", original);
                }
                if let Ok(original) = original_xdg {
                    env::set_var("XDG_CONFIG_HOME", original);
                } else {
                    env::remove_var("XDG_CONFIG_HOME");
                }
            }
        }
    }

    #[tokio::test]
    async fn test_get_config_dir_cross_platform() {
        let result = get_config_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains("turbogrep"));

        // Verify path format based on OS
        if cfg!(target_os = "macos") {
            // macOS now uses XDG standard like other Unix systems
            assert!(
                path.to_string_lossy().contains(".config") ||
                path.to_string_lossy().contains("XDG_CONFIG_HOME")
            );
        } else if cfg!(target_os = "windows") {
            // On Windows, we expect APPDATA path
            assert!(path.is_absolute());
        } else {
            // Unix-like systems should have .config or XDG_CONFIG_HOME
            assert!(path.to_string_lossy().contains("turbogrep"));
        }
    }
}
