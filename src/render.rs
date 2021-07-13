use std::{
    collections::HashSet,
    fs::{self, File},
    io::BufReader,
    path::PathBuf,
};

use anyhow::{anyhow, Context, Result};
use bincode::Options;
use chrono::NaiveDate;
use itertools::{Itertools, MinMaxResult};
use plotters::prelude::*;
use rayon::prelude::*;
use svgcleaner::{cleaner, CleaningOptions, ParseOptions, StyleJoinMode, WriteOptions};
use svgdom::{AttributesOrder, Indent, ListSeparator};
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
    let mut buf = String::new();
    let root = SVGBackend::with_string(&mut buf, size).into_drawing_area();
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

    drop(chart);
    drop(root);

    println!("optimizing...");

    let buf = optimize(&buf)?;
    fs::write("stats.svg", &buf)?;

    println!("done");

    Ok(())
}

fn load_data(input: PathBuf, filter: &HashSet<LanguageType>) -> Result<Vec<SimpleEntry>> {
    let bincode = bincode::DefaultOptions::new().allow_trailing_bytes();

    let (total_entries, file_count) = {
        let input = BufReader::new(File::open(&input)?);
        let mut input = ZipArchive::new(input)?;

        let count = input.len() - 1;
        let file = input.by_index_raw(0)?;
        let mut file = BufReader::new(ZstdDecoder::new(file)?);

        (bincode.deserialize_from::<_, u64>(&mut file)?, count)
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
            let count = bincode.deserialize_from::<_, u64>(&mut reader)?;

            list.reserve(count as usize);

            for _ in 0..count {
                let entry = bincode.deserialize_from::<_, Entry>(&mut reader)?;
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

fn minmax_value<T: Copy>(mmr: MinMaxResult<T>) -> Option<(T, T)> {
    match mmr {
        MinMaxResult::NoElements => None,
        MinMaxResult::OneElement(v) => Some((v, v)),
        MinMaxResult::MinMax(min, max) => Some((min, max)),
    }
}

fn optimize(data: &str) -> Result<Vec<u8>> {
    let parse_options = ParseOptions {
        parse_comments: true,
        parse_declarations: true,
        parse_unknown_elements: true,
        parse_unknown_attributes: true,
        parse_px_unit: false,
        skip_unresolved_classes: true,
        skip_invalid_attributes: false,
        skip_invalid_css: false,
        skip_paint_fallback: false,
    };
    let cleaning_options = CleaningOptions {
        remove_unused_defs: true,
        convert_shapes: true,
        remove_title: true,
        remove_desc: true,
        remove_metadata: true,
        remove_dupl_linear_gradients: true,
        remove_dupl_radial_gradients: true,
        remove_dupl_fe_gaussian_blur: true,
        ungroup_groups: true,
        ungroup_defs: true,
        group_by_style: true,
        merge_gradients: true,
        regroup_gradient_stops: true,
        remove_invalid_stops: true,
        remove_invisible_elements: true,
        resolve_use: true,
        remove_version: true,
        remove_unreferenced_ids: true,
        trim_ids: true,
        remove_text_attributes: true,
        remove_unused_coordinates: true,
        remove_default_attributes: true,
        remove_xmlns_xlink_attribute: true,
        remove_needless_attributes: true,
        remove_gradient_attributes: false,
        join_style_attributes: StyleJoinMode::Some,
        apply_transform_to_gradients: true,
        apply_transform_to_shapes: true,
        paths_to_relative: true,
        remove_unused_segments: true,
        convert_segments: true,
        apply_transform_to_paths: false,
        coordinates_precision: 6,
        properties_precision: 6,
        paths_coordinates_precision: 8,
        transforms_precision: 8,
    };
    let write_options = WriteOptions {
        indent: Indent::None,
        attributes_indent: Indent::None,
        use_single_quote: false,
        trim_hex_colors: true,
        write_hidden_attributes: false,
        remove_leading_zero: true,
        use_compact_path_notation: true,
        join_arc_to_flags: false,
        remove_duplicated_path_commands: true,
        use_implicit_lineto_commands: true,
        simplify_transform_matrices: true,
        list_separator: ListSeparator::Space,
        attributes_order: AttributesOrder::Alphabetical,
    };

    let mut document = cleaner::parse_data(data, &parse_options)
        .map_err(|e| anyhow!("failed parsing SVG document: {}", e))?;

    cleaner::clean_doc(&mut document, &cleaning_options, &write_options)
        .map_err(|e| anyhow!("failed cleaning up SVG document: {}", e))?;

    let mut buf = Vec::new();
    cleaner::write_buffer(&document, &write_options, &mut buf);

    Ok(buf)
}
