use std::{
    collections::HashSet,
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};

use anyhow::{Context, Result};
use chrono::NaiveDate;
use itertools::{Itertools, MinMaxResult};
use plotters::prelude::*;
use tokei::LanguageType;
use zstd::Decoder as ZstdDecoder;

use crate::models::Entry;

struct SimpleEntry {
    timestamp: NaiveDate,
    code: u64,
    comments: u64,
}

pub fn run(mut filter: Vec<LanguageType>, input: PathBuf,size:(u32,u32)) -> Result<()> {
    let root = SVGBackend::new("test.svg", size).into_drawing_area();
    root.fill(&WHITE)?;

    if filter.is_empty() {
        filter = LanguageType::list().to_owned();
    }

    println!("loading input data...");

    let filter = filter.into_iter().collect::<HashSet<_>>();
    let data = load_data(input, &filter)?;

    let (min_x, max_x) =
        minmax_value(data.iter().map(|e| e.timestamp).minmax()).context("no data")?;
    let max_code = data.iter().map(|e| e.code).max().context("no data")?;
    let max_comments = data.iter().map(|e| e.comments).max().context("no data")?;
    let max_y = max_code.max(max_comments);

    println!("rendering...");

    let mut chart = ChartBuilder::on(&root)
        .x_label_area_size(80)
        .y_label_area_size(80)
        .caption("Code over time", ("sans-serif", 50).into_font())
        .margin(15)
        .build_cartesian_2d(min_x..max_x, 0..max_y)?;

    chart.configure_mesh().light_line_style(&WHITE).draw()?;

    chart
        .draw_series(LineSeries::new(
            data.iter().map(|e| (e.timestamp, e.code)),
            &BLUE,
        ))?
        .label("Code")
        .legend(|(x, y)| Rectangle::new([(x, y - 1), (x + 20, y + 1)], BLUE.filled()));

    chart
        .draw_series(LineSeries::new(
            data.iter().map(|e| (e.timestamp, e.comments)),
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

    println!("done");

    Ok(())
}

fn load_data(input: PathBuf, filter: &HashSet<LanguageType>) -> Result<Vec<SimpleEntry>> {
    let input = BufReader::new(ZstdDecoder::new(File::open(input)?)?);

    input
        .lines()
        .map(|line| {
            let entry = serde_json::from_str::<Entry>(&line?)?;
            let filtered = entry
                .filtered(filter)
                .fold((0, 0), |acc, cs| (acc.0 + cs.code, acc.1 + cs.comments));

            Ok(SimpleEntry {
                timestamp: entry.timestamp.date().naive_local(),
                code: filtered.0 as u64,
                comments: filtered.1 as u64,
            })
        })
        .collect()
}

fn minmax_value<T: Copy>(mmr: MinMaxResult<T>) -> Option<(T, T)> {
    match mmr {
        MinMaxResult::NoElements => None,
        MinMaxResult::OneElement(v) => Some((v, v)),
        MinMaxResult::MinMax(min, max) => Some((min, max)),
    }
}
