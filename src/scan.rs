use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use anyhow::{anyhow, Result};
use chrono::prelude::*;
use git2::{Delta, ObjectType, Oid, Repository, Sort, Tree};
use pbr::ProgressBar;
use tokei::{Config as TokeiConfig, LanguageType};
use zstd::Encoder as ZstdEncoder;

use crate::models::{Entry, EntryFile};

pub fn run(input: PathBuf) -> Result<()> {
    let repo = Repository::open(&input)?;
    let mut walk = repo.revwalk()?;

    walk.push_head()?;
    walk.set_sorting(Sort::TIME | Sort::REVERSE)?;

    let oids = walk
        .map(|oid| oid.map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;

    let total = oids.len() as u64;
    let progress = Arc::new(AtomicU64::new(0));
    let progress2 = Arc::clone(&progress);
    let mut pb = ProgressBar::new(total);

    let handle = thread::spawn(move || loop {
        let p = progress2.load(Ordering::SeqCst);
        if p >= total {
            pb.finish();
            break;
        }

        pb.set(p);
        thread::sleep(Duration::from_millis(200));
    });

    let mut file = ZstdEncoder::new(BufWriter::new(File::create("stats.json.zst")?), 11)?;
    file.multithread(num_cpus::get() as u32)?;
    file.include_checksum(true)?;
    file.include_contentsize(true)?;

    let mut previous_entry = None;
    let mut previous_tree = None;

    for oid in oids {
        let (entry, tree) = commit_stats(&repo, oid, previous_entry, previous_tree, &progress)?;

        serde_json::to_writer(&mut file, &entry)?;
        file.write_all(&[b'\n'])?;

        previous_entry = Some(entry);
        previous_tree = Some(tree);
    }

    file.try_finish().map_err(|(_, e)| e)?.flush()?;

    handle
        .join()
        .map_err(|_| anyhow!("failed joining progress printer thread"))?;

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

    progress.fetch_add(1, Ordering::SeqCst);

    Ok((entry, tree))
}
