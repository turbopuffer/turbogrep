use std::sync::OnceLock;
use std::time::Instant;

static VERBOSE: OnceLock<bool> = OnceLock::new();
pub static START_TIME: OnceLock<Instant> = OnceLock::new();

pub fn is_verbose() -> bool {
    // TURBOGREP_VERBOSE environment variable OR TG_VERBOSE
    for var in ["TURBOGREP_VERBOSE", "TG_VERBOSE"] {
        if let Ok(verbose) = std::env::var(var) {
            return verbose == "1" || verbose.to_lowercase() == "true";
        }
    }
    *VERBOSE.get().unwrap_or(&false)
}

pub fn set_verbose(verbose: bool) {
    VERBOSE.set(verbose).ok();
}

#[macro_export]
macro_rules! vprintln {
    ($($arg:tt)*) => {
        if $crate::is_verbose() {
            let start_time = $crate::START_TIME.get_or_init(std::time::Instant::now);
            let elapsed = start_time.elapsed();
            println!("{:.3}s {}", elapsed.as_secs_f64(), format!($($arg)*));
        }
    };
}

// Re-export project functions for backward compatibility
pub use project::{find_project_root, namespace_and_dir, validate_directory};

// Re-export progress bar function for backward compatibility
pub use progress::tg_progress_bar;

pub mod chunker;
pub mod config;
pub mod embeddings;
pub mod progress;
pub mod project;
pub mod search;
pub mod sync;
pub mod turbopuffer;
