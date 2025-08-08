use indicatif::{ProgressBar, ProgressStyle};

/// Create a standard TurboPuffer-branded progress bar with consistent styling
/// Follows TurboPuffer brand guidelines from https://turbopuffer.com/press
pub fn tg_progress_bar(total: u64) -> ProgressBar {
    if !crate::is_verbose() {
        return ProgressBar::hidden();
    }

    let pb = ProgressBar::new(total);
    pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "<(°O°)> {spinner:.cyan} [{elapsed_precise}] [{bar:38.cyan/blue}] {pos}/{len} ({per_sec}) turbopuffer"
                )
                .unwrap()
                .progress_chars("#>-")
                .tick_strings(&[
                    "<(°o°)>", 
                    "<(°O°)>", 
                    "<(°◯°)>", 
                    "<(°O°)>", 
                    "<(°o°)>",
                    "<(°◯°)>",
                    "<(°○°)>"
                ]),
        );
    pb
}

