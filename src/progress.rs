use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{anyhow, Result};
use pbr::ProgressBar;

pub struct Progress {
    handle: JoinHandle<()>,
}

impl Progress {
    pub fn new(total: u64) -> (Self, Updater) {
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

        (Self { handle }, Updater { progress })
    }

    pub fn wait(self) -> Result<()> {
        self.handle
            .join()
            .map_err(|_| anyhow!("failed joining progress printer thread"))
    }
}

#[derive(Clone)]
pub struct Updater {
    progress: Arc<AtomicU64>,
}

impl Updater {
    pub fn inc(&self) {
        self.progress.fetch_add(1, Ordering::Relaxed);
    }
}
