use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use tokei::{CodeStats, LanguageType};

#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub timestamp: DateTime<FixedOffset>,
    pub files: HashMap<PathBuf, EntryFile>,
}

impl Entry {
    pub fn filtered<'a>(
        &'a self,
        filter: &'a HashSet<LanguageType>,
    ) -> impl Iterator<Item = &'a CodeStats> {
        self.files.iter().filter_map(move |(_, v)| {
            if filter.contains(&v.language) {
                Some(&v.statistics)
            } else {
                None
            }
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct EntryFile {
    pub language: LanguageType,
    pub statistics: CodeStats,
}
