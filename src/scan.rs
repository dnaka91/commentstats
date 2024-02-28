use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use chrono::prelude::*;
use git2::{Delta, ObjectType, Oid, Repository, Sort, Tree};
use pbr::ProgressBar;
use rayon::prelude::*;
use tokei::{Config as TokeiConfig, LanguageType};
use zip::{write::FileOptions, ZipWriter};
use zstd::Encoder as ZstdEncoder;

use crate::{
    models::{Entry, EntryFile},
    progress::{Progress, Updater},
};

/// The amount of chunks to create. This is a _goal_ value that means if there is not enough data
/// it may be less.
const CHUNK_AMOUNT: usize = 1000;
/// Minimum amount of items in a single chunk. The value seems to be a good trade between overhead
/// and work split amount. Small repositories can be handled quickly so that mostly bigger repos
/// benefit from the chunking anyways.
const MIN_CHUNK_SIZE: usize = 1000;
const ZSTD_COMPRESSION_DEFAULT: i32 = 11;

pub fn run(input: PathBuf) -> Result<()> {
    let repo = Repository::open(&input)?;
    let mut walk = repo.revwalk()?;

    println!("reading history...");

    walk.push_head()?;
    walk.set_sorting(Sort::TIME | Sort::REVERSE)?;

    let oids = walk
        .map(|oid| oid.map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;

    let dir = tempfile::tempdir()?;
    let config = bincode::config::standard();

    let mut info_file = new_zstd_file(dir.path().join("info"))?;
    bincode::encode_into_std_write(oids.len() as u64, &mut info_file, config)?;
    info_file.finish()?.flush()?;

    println!("scanning...");

    let (progress, updater) = Progress::new(oids.len() as u64);

    let chunk_size = MIN_CHUNK_SIZE.max(oids.len() / CHUNK_AMOUNT);

    oids.par_chunks(chunk_size).enumerate().try_for_each_init(
        || Repository::open(&input),
        |repo, (i, chunk)| -> Result<()> {
            let repo = repo.as_ref().map_err(|e| anyhow!("{}", e))?;

            let mut file = new_zstd_file(dir.path().join(format!("stats-{:03}", i,)))?;
            bincode::encode_into_std_write(chunk.len() as u64, &mut file, config)?;

            let mut previous_entry = None;
            let mut previous_tree = None;

            for &oid in chunk {
                let (entry, tree) =
                    commit_stats(repo, oid, previous_entry, previous_tree, &updater)?;

                bincode::serde::encode_into_std_write(&entry, &mut file, config)?;

                previous_entry = Some(entry);
                previous_tree = Some(tree);
            }

            file.finish()?.flush()?;

            Ok(())
        },
    )?;

    progress.wait()?;

    println!("saving statistics...");

    let mut files = std::fs::read_dir(dir.path())?
        .map(|r| r.map(|e| e.path()).map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;

    files.sort();

    let mut zip_file = ZipWriter::new(BufWriter::new(File::create("stats.stats")?));
    let mut pb = ProgressBar::new(files.len() as u64);
    pb.set_width(Some(80));

    for path in &files {
        let mut file = File::open(path)?;
        let name = path.file_name().unwrap().to_string_lossy();

        zip_file.start_file(name, FileOptions::default())?;
        io::copy(&mut file, &mut zip_file)?;

        pb.inc();
    }

    zip_file.finish()?.flush()?;
    pb.finish();

    Ok(())
}

fn commit_stats<'a>(
    repo: &'a Repository,
    oid: Oid,
    previous_entry: Option<Entry>,
    previous_tree: Option<Tree<'_>>,
    updater: &Updater,
) -> Result<(Entry, Tree<'a>)> {
    let config = TokeiConfig::default();
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;
    let time = commit.time();
    let time = FixedOffset::east_opt(time.offset_minutes() * 60)
        .context("offset out of bounds")?
        .from_utc_datetime(
            &NaiveDateTime::from_timestamp_opt(time.seconds(), 0)
                .context("timestamp out of bounds")?,
        );

    let diff = repo.diff_tree_to_tree(previous_tree.as_ref(), Some(&tree), None)?;
    let mut entry = Entry {
        timestamp: time,
        files: previous_entry.map(|e| e.files).unwrap_or_default(),
    };

    for delta in diff.deltas() {
        match delta.status() {
            Delta::Added | Delta::Modified => {
                let item = tree.get_path(delta.new_file().path().unwrap()).unwrap();

                if matches!(item.kind(), Some(ObjectType::Blob)) {
                    let name = item.name().unwrap_or_default();
                    let lang = LanguageType::from_path(name, &config);

                    if let Some(lang) = lang {
                        let blob = item
                            .to_object(repo)?
                            .into_blob()
                            .map_err(|_| anyhow!("not a blob"))?;

                        let stats = lang.parse_from_slice(blob.content(), &config);
                        let stats = stats.summarise();

                        entry.files.insert(
                            delta.new_file().path().unwrap().to_owned(),
                            EntryFile {
                                language: lang,
                                statistics: stats,
                            },
                        );
                    }
                }
            }
            Delta::Deleted => {
                entry.files.remove(delta.old_file().path().unwrap());
            }
            Delta::Renamed => {
                let old = entry
                    .files
                    .remove(delta.old_file().path().unwrap())
                    .unwrap();
                entry
                    .files
                    .insert(delta.new_file().path().unwrap().to_owned(), old);
            }
            Delta::Copied => {
                let old = entry
                    .files
                    .get(delta.old_file().path().unwrap())
                    .unwrap()
                    .clone();
                entry
                    .files
                    .insert(delta.new_file().path().unwrap().to_owned(), old);
            }
            _ => unreachable!(),
        }
    }

    updater.inc();

    Ok((entry, tree))
}

fn new_zstd_file<'a>(path: impl AsRef<Path>) -> Result<ZstdEncoder<'a, BufWriter<File>>> {
    ZstdEncoder::new(
        BufWriter::new(File::create(path.as_ref())?),
        ZSTD_COMPRESSION_DEFAULT,
    )
    .map_err(Into::into)
}
