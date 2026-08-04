//! Command-line arguments for the engine.

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Args {
    /// Window title
    #[arg(long, default_value = "Resident")]
    pub title: String,

    /// Window width in logical pixels
    #[arg(long, default_value_t = 1280)]
    pub width: u32,

    /// Window height in logical pixels
    #[arg(long, default_value_t = 720)]
    pub height: u32,

    /// Disable vsync
    #[arg(long)]
    pub no_vsync: bool,
}
