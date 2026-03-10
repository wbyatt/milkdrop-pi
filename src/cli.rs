use clap::Parser;

#[derive(Parser)]
#[command(name = "milkdrop-pi", about = "Audio visualizer for broken CRTs")]
pub struct Args {
    /// Comma-separated visualization names, or "all" (default: all).
    #[arg(long, default_value = "all")]
    pub viz: String,

    /// Seconds each visualization is shown before cycling to the next.
    #[arg(long, default_value_t = 60)]
    pub duration: u64,
}

impl Args {
    pub fn viz_names(&self) -> Vec<String> {
        if self.viz.eq_ignore_ascii_case("all") {
            return Vec::new(); // empty signals "use all"
        }
        self.viz.split(',').map(|s| s.trim().to_lowercase()).collect()
    }
}
