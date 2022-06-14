use std::path::PathBuf;

use anyhow::Result;
use clap::{AppSettings, Parser, Subcommand};
use tokei::LanguageType;

mod list_filters;
mod models;
mod progress;
mod render;
mod scan;

/// Generate statistical graphs about the code/comment rate in code repositories.
#[derive(Parser)]
#[clap(
    about,
    author,
    version,
    global_setting = AppSettings::DeriveDisplayOrder,
)]
struct Opt {
    #[clap(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List all possible languages that can be used as filters.
    ListFilters,
    /// Scan a repository and generate statistics.
    Scan {
        /// Target Git repository.
        #[clap(value_parser)]
        input: PathBuf,
    },
    /// Load statistics from a pre-generated `stats.json` file.
    Render {
        /// Output image width.
        #[clap(long, value_parser, default_value_t = 1600)]
        width: u32,
        /// Output image height.
        #[clap(long, value_parser, default_value_t = 1000)]
        height: u32,
        /// One or more languages to filter the plotting output with.
        #[clap(short, long, value_parser)]
        filter: Vec<LanguageType>,
        /// Location fo the statistics file.
        #[clap(value_parser)]
        input: PathBuf,
    },
}

fn main() -> Result<()> {
    let opt = Opt::parse();

    match opt.cmd {
        Command::ListFilters => list_filters::run(),
        Command::Scan { input } => scan::run(input)?,
        Command::Render {
            filter,
            input,
            width,
            height,
        } => render::run(filter, input, (width, height))?,
    }

    Ok(())
}
