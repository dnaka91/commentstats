use std::path::PathBuf;

use anyhow::Result;
use structopt::{clap::AppSettings, StructOpt};
use tokei::LanguageType;

mod list_filters;
mod models;
mod render;
mod scan;

/// Generate statistical graphs about the code/comment rate in code repositories.
#[derive(StructOpt)]
#[structopt(global_setting = AppSettings::ColoredHelp)]
struct Opt {
    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(StructOpt)]
enum Command {
    /// List all possible languages that can be used as filters.
    ListFilters,
    /// Scan a repository and generate statistics.
    Scan {
        /// Target Git repository.
        input: PathBuf,
    },
    /// Load statistics from a pre-generated `stats.json` file.
    Render {
        /// One or more languages to filter the plotting output with.
        #[structopt(short, long)]
        filter: Vec<LanguageType>,
        /// Location fo the statistics file.
        input: PathBuf,
    },
}

fn main() -> Result<()> {
    let opt = Opt::from_args_safe()?;

    match opt.cmd {
        Command::ListFilters => list_filters::run(),
        Command::Scan { input } => scan::run(input)?,
        Command::Render { filter, input } => render::run(filter, input)?,
    }

    Ok(())
}
