use std::path::PathBuf;

use anyhow::Result;
use structopt::{clap::AppSettings, StructOpt};
use tokei::LanguageType;

mod list_filters;
mod models;
mod progress;
mod render;
mod scan;

/// Generate statistical graphs about the code/comment rate in code repositories.
#[derive(StructOpt)]
#[structopt(
    author,
    global_setting = AppSettings::ColoredHelp,
    global_setting = AppSettings::DeriveDisplayOrder,
)]
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
        /// Output image width.
        #[structopt(long, default_value = "1600")]
        width: u32,
        /// Output image height.
        #[structopt(long, default_value = "1000")]
        height: u32,
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
        Command::Render {
            filter,
            input,
            width,
            height,
        } => render::run(filter, input, (width, height))?,
    }

    Ok(())
}
