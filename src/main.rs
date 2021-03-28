use std::{
    collections::{HashMap, HashSet},
    env,
    fs::File,
    path::PathBuf,
};

use anyhow::{anyhow, Context, Result};
use chrono::prelude::*;
use git2::{ObjectType, Oid, Repository, Sort, TreeWalkMode, TreeWalkResult};
use itertools::{Itertools, MinMaxResult};
use plotters::prelude::*;
use serde::{Deserialize, Serialize};
use structopt::{clap::AppSettings, StructOpt};
use tokei::{CodeStats, Config, LanguageType};

/// Generate statistical graphs about the code/comment rate in code repositories.
#[derive(StructOpt)]
#[structopt(setting = AppSettings::ColoredHelp)]
struct Opt {
    /// Load statistics from a pre-generated `stats.json` file.
    #[structopt(short, long)]
    statsfile: bool,
    /// List all possible languages that can be used as filters.
    #[structopt(long)]
    list_filters: bool,
    /// One or more languages to filter the plotting output with.
    #[structopt(long)]
    filter: Vec<LanguageType>,
    /// Target repository or if `--statsfile` is passed the location of a `stats.json` file.
    #[structopt(required_if("statsfile", "true"))]
    input: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    timestamp: DateTime<FixedOffset>,
    statistics: HashMap<LanguageType, CodeStats>,
}

impl Entry {
    fn filtered<'a>(
        &'a self,
        filter: &'a HashSet<LanguageType>,
    ) -> impl Iterator<Item = &'a CodeStats> {
        self.statistics
            .iter()
            .filter_map(move |(k, v)| if filter.contains(k) { Some(v) } else { None })
    }
}

fn main() -> Result<()> {
    let opt = Opt::from_args_safe()?;

    if opt.list_filters {
        for filter in LanguageType::list() {
            println!("{:?}", filter);
        }

        return Ok(());
    }

    let input = opt
        .input
        .or_else(|| env::current_dir().ok())
        .context("no input")?;

    let data = if opt.statsfile {
        let file = File::open(input)?;
        serde_json::from_reader(file)?
    } else {
        let repo = Repository::open(input)?;
        let mut walk = repo.revwalk()?;

        walk.push_head()?;
        walk.set_sorting(Sort::TIME | Sort::REVERSE)?;

        let data = walk
            .map(|oid| commit_stats(&repo, oid?))
            .collect::<Result<Vec<_>>>()?;

        let file = File::create("stats.json")?;
        serde_json::to_writer_pretty(file, &data)?;

        data
    };

    let root = BitMapBackend::new("test.png", (1920, 1080)).into_drawing_area();
    root.fill(&WHITE)?;

    let mut filter = opt.filter;
    if filter.is_empty() {
        filter = LanguageType::list().to_owned();
    }

    let filter = filter.into_iter().collect::<HashSet<_>>();

    let (min_date, max_date) =
        minmax_value(data.iter().map(|d| d.timestamp).minmax()).context("no data")?;
    let max_code = data
        .iter()
        .filter_map(|d| d.filtered(&filter).map(|v| v.code).max())
        .max()
        .context("no data")?;
    let max_comments = data
        .iter()
        .filter_map(|d| d.filtered(&filter).map(|v| v.comments).max())
        .max()
        .context("no data")?;
    let min_x = min_date.date().naive_local();
    let max_x = max_date.date().naive_local();
    let max_y = max_code.max(max_comments);

    let mut chart = ChartBuilder::on(&root)
        .x_label_area_size(80)
        .y_label_area_size(80)
        .caption("Code over time", ("sans-serif", 50).into_font())
        .margin(15)
        .build_cartesian_2d(min_x..max_x, 0..max_y)?;

    chart.configure_mesh().light_line_style(&WHITE).draw()?;

    chart
        .draw_series(LineSeries::new(
            data.iter().map(|d| {
                (
                    d.timestamp.date().naive_local(),
                    d.filtered(&filter).map(|s| s.code).sum(),
                )
            }),
            &BLUE,
        ))?
        .label("Code")
        .legend(|(x, y)| Rectangle::new([(x, y - 1), (x + 20, y + 1)], BLUE.filled()));

    chart
        .draw_series(LineSeries::new(
            data.iter().map(|d| {
                (
                    d.timestamp.date().naive_local(),
                    d.filtered(&filter).map(|s| s.comments).sum(),
                )
            }),
            &RED,
        ))?
        .label("Comments")
        .legend(|(x, y)| Rectangle::new([(x, y - 1), (x + 20, y + 1)], RED.filled()));

    chart
        .configure_series_labels()
        .background_style(&WHITE.mix(0.8))
        .border_style(&BLACK)
        .label_font(("sans-serif", 20).into_font())
        .draw()?;

    Ok(())
}

fn commit_stats(repo: &Repository, oid: Oid) -> Result<Entry> {
    let config = Config::default();
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;
    let time = commit.time();
    let time = FixedOffset::east(time.offset_minutes() * 60)
        .from_utc_datetime(&NaiveDateTime::from_timestamp(time.seconds(), 0));

    let mut statistics = HashMap::new();

    println!("{}", time);

    tree.walk(TreeWalkMode::PreOrder, |_, entry| {
        if !matches!(
            entry.kind(),
            Some(ObjectType::Blob) | Some(ObjectType::Tree)
        ) {
            return TreeWalkResult::Skip;
        }

        if matches!(entry.kind(), Some(ObjectType::Blob)) {
            let name = entry.name().unwrap_or_default();
            let lang = LanguageType::from_path(name, &config);

            if let Some(lang) = lang {
                let blob = match entry
                    .to_object(&repo)
                    .context("not an object")
                    .and_then(|o| o.into_blob().map_err(|_| anyhow!("not a blob")))
                {
                    Ok(blob) => blob,
                    Err(_) => return TreeWalkResult::Ok,
                };

                let stats = lang.parse_from_slice(blob.content(), &config);
                let stats = stats.summarise();

                let commit_stats = statistics.entry(lang).or_default();
                *commit_stats += stats;
            }
        }

        TreeWalkResult::Ok
    })?;

    Ok(Entry {
        timestamp: time,
        statistics,
    })
}

fn minmax_value<T: Copy>(mmr: MinMaxResult<T>) -> Option<(T, T)> {
    match mmr {
        MinMaxResult::NoElements => None,
        MinMaxResult::OneElement(v) => Some((v, v)),
        MinMaxResult::MinMax(min, max) => Some((min, max)),
    }
}
