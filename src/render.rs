use std::{
    collections::HashSet,
    fmt::Write,
    fs::{self, File},
    io::BufReader,
    path::PathBuf,
};

use anyhow::Result;
use chrono::{NaiveDate, TimeZone};
use poloto::{num::timestamp::UnixTime, prelude::*};
use rayon::prelude::*;
use tokei::LanguageType;
use zip::ZipArchive;
use zstd::Decoder as ZstdDecoder;

use crate::{models::Entry, progress::Progress};

struct SimpleEntry {
    timestamp: NaiveDate,
    code: u64,
    comments: u64,
}

pub fn run(mut filter: Vec<LanguageType>, input: PathBuf, size: (u32, u32)) -> Result<()> {
    if filter.is_empty() {
        filter = LanguageType::list().to_owned();
    }

    println!("loading input data...");

    let filter = filter.into_iter().collect::<HashSet<_>>();
    let data = load_data(input, &filter)?;

    println!("rendering...");

    let opt = poloto::render::render_opt_builder()
        .with_tick_lines([true, true])
        .with_dim([size.0 as f64, size.1 as f64])
        .build();

    let plotter = quick_fmt_opt!(
        opt,
        "Code over time",
        "Date",
        "Lines",
        poloto::build::markers([], [0.0]),
        data.iter()
            .map(|e| (
                UnixTime::from(chrono::Utc.from_utc_date(&e.timestamp)),
                e.code as f64
            ))
            .cloned_plot()
            .line("Code"),
        data.iter()
            .map(|e| (
                UnixTime::from(chrono::Utc.from_utc_date(&e.timestamp)),
                e.comments as f64
            ))
            .cloned_plot()
            .line("Comments")
    );

    let mut buf = String::new();
    write!(buf, "{}", poloto::disp(|w| plotter.simple_theme(w)))?;

    fs::write("stats.svg", &buf)?;

    println!("done");

    Ok(())
}

fn load_data(input: PathBuf, filter: &HashSet<LanguageType>) -> Result<Vec<SimpleEntry>> {
    let config = bincode::config::standard();

    let (total_entries, file_count) = {
        let input = BufReader::new(File::open(&input)?);
        let mut input = ZipArchive::new(input)?;

        let count = input.len() - 1;
        let file = input.by_index_raw(0)?;
        let mut file = BufReader::new(ZstdDecoder::new(file)?);

        (
            bincode::decode_from_std_read::<u64, _, _>(&mut file, config)?,
            count,
        )
    };

    println!("processing data...");

    let (progress, updater) = Progress::new(total_entries);

    let data = (1..file_count + 1)
        .into_par_iter()
        .try_fold(Vec::new, |mut list, i| {
            let input = BufReader::new(File::open(&input)?);
            let mut input = ZipArchive::new(input)?;
            let file = input.by_index_raw(i)?;

            let mut reader = ZstdDecoder::new(file)?;
            let count = bincode::decode_from_std_read::<u64, _, _>(&mut reader, config)?;

            list.reserve(count as usize);

            for _ in 0..count {
                let entry =
                    bincode::serde::decode_from_std_read::<Entry, _, _>(&mut reader, config)?;
                let filtered = entry
                    .filtered(filter)
                    .fold((0, 0), |acc, cs| (acc.0 + cs.code, acc.1 + cs.comments));

                list.push(SimpleEntry {
                    timestamp: entry.timestamp.date().naive_local(),
                    code: filtered.0 as u64,
                    comments: filtered.1 as u64,
                });

                updater.inc();
            }

            Ok(list)
        })
        .try_reduce(Vec::new, |mut list, sublist| {
            list.extend(sublist);
            Ok(list)
        });

    progress.wait()?;

    data
}
