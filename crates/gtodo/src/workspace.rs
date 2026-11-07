//! `data_dir` を読み込んだ作業環境。

use std::path::PathBuf;

use anyhow::{Result, bail};
use chrono::NaiveDate;
use gtodo_core::document::Document;
use gtodo_core::sections::SectionNames;
use gtodo_core::store::{self, DataDir, FileKind};

pub struct Workspace {
    pub data_dir: DataDir,
    pub names: SectionNames,
    pub today: NaiveDate,
}

impl Workspace {
    pub fn open(data_dir: PathBuf, today: NaiveDate) -> Result<Workspace> {
        if !data_dir.is_dir() {
            bail!(
                "data directory not found: {} (set --data-dir, GTODO_DATA_DIR or data_dir in config.json)",
                data_dir.display()
            );
        }
        let data_dir = DataDir::new(data_dir);
        let names = SectionNames::default();
        Ok(Workspace { data_dir, names, today })
    }

    pub fn load(&self, kind: FileKind) -> Result<Document> {
        let lines = store::read_lines(&self.data_dir.path(kind))?;
        Ok(Document::parse(&lines, &self.names, self.today))
    }
}
