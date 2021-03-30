use std::{
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use anyhow::{anyhow, Result};
use bincode::Options;
use chrono::prelude::*;
use git2::{Delta, ObjectType, Oid, Repository, Sort, Tree};
use pbr::ProgressBar;
use rayon::prelude::*;
use tokei::{Config as TokeiConfig, LanguageType};
use zip::{write::FileOptions, ZipWriter};
use zstd::Encoder as ZstdEncoder;

use crate::models::{Entry, EntryFile};

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
    let bincode = bincode::DefaultOptions::new().allow_trailing_bytes();

    let mut info_file = new_zstd_file(dir.path().join("info"))?;
    bincode.serialize_into(&mut info_file, &(oids.len() as u64))?;
    info_file.finish()?.flush()?;

    println!("scanning...");

    let total = oids.len() as u64;
    let progress = Arc::new(AtomicU64::new(0));
    let progress2 = Arc::clone(&progress);
    let mut pb = ProgressBar::new(total);
    pb.set_width(Some(80));

    let handle = thread::spawn(move || loop {
        let p = progress2.load(Ordering::Relaxed);
        if p >= total {
            pb.finish();
            println!();
            break;
        }

        pb.set(p);
        thread::sleep(Duration::from_millis(200));
    });

    let chunk_size = MIN_CHUNK_SIZE.max(oids.len() / CHUNK_AMOUNT);

    oids.par_chunks(chunk_size).enumerate().try_for_each_init(
        || Repository::open(&input),
        |repo, (i, chunk)| -> Result<()> {
            let repo = repo.as_ref().map_err(|e| anyhow!("{}", e))?;

            let mut file = new_zstd_file(dir.path().join(format!("stats-{:03}", i,)))?;
            bincode.serialize_into(&mut file, &(chunk.len() as u64))?;

            let mut previous_entry = None;
            let mut previous_tree = None;

            for &oid in chunk {
                let (entry, tree) =
                    commit_stats(&repo, oid, previous_entry, previous_tree, &progress)?;

                bincode.serialize_into(&mut file, &entry)?;

                previous_entry = Some(entry);
                previous_tree = Some(tree);
            }

            file.finish()?.flush()?;

            Ok(())
        },
    )?;

    handle
        .join()
        .map_err(|_| anyhow!("failed joining progress printer thread"))?;

    println!("saving statistics...");

    let mut files = std::fs::read_dir(dir.path())?
        .map(|r| r.map(|e| e.path()).map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;

    files.sort();

    let mut zip_file = ZipWriter::new(BufWriter::new(File::create("stats.stats")?));
    let mut pb = ProgressBar::new(files.len() as u64);
    pb.set_width(Some(80));

    for path in &files {
        let mut file = File::open(&path)?;
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
    progress: &Arc<AtomicU64>,
) -> Result<(Entry, Tree<'a>)> {
    let config = TokeiConfig::default();
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;
    let time = commit.time();
    let time = FixedOffset::east(time.offset_minutes() * 60)
        .from_utc_datetime(&NaiveDateTime::from_timestamp(time.seconds(), 0));

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
                            .to_object(&repo)?
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

    progress.fetch_add(1, Ordering::Relaxed);

    Ok((entry, tree))
}

fn new_zstd_file<'a>(path: impl AsRef<Path>) -> Result<ZstdEncoder<'a, BufWriter<File>>> {
    ZstdEncoder::new(
        BufWriter::new(File::create(path.as_ref())?),
        ZSTD_COMPRESSION_DEFAULT,
    )
    .map_err(Into::into)
}
