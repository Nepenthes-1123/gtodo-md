//! `data_dir` とその設定を読み込んだ作業環境。

use std::path::PathBuf;

use anyhow::{Result, bail};
use chrono::NaiveDate;
use gtodo_core::document::Document;
use gtodo_core::sections::SectionNames;
use gtodo_core::settings;
use gtodo_core::store::{self, DataDir, FileKind};

pub struct Workspace {
    pub data_dir: DataDir,
    pub names: SectionNames,
    pub today: NaiveDate,
}

impl Workspace {
    /// `warn` には、処理を続けられるが利用者に知らせるべき問題が渡される。
    pub fn open(data_dir: PathBuf, today: NaiveDate, warn: &mut impl FnMut(String)) -> Result<Workspace> {
        if !data_dir.is_dir() {
            bail!(
                "data directory not found: {} (set --data-dir, GTODO_DATA_DIR or data_dir in config.json)",
                data_dir.display()
            );
        }
        let data_dir = DataDir::new(data_dir);
        // セクション名が分からないとタスクを別のセクションとして読み違えるため、壊れた gtodo.json では止める。
        let settings = settings::read(&data_dir.settings_file(), warn)?;
        let names = settings.sections;
        Ok(Workspace { data_dir, names, today })
    }

    pub fn load(&self, kind: FileKind) -> Result<Document> {
        let lines = store::read_lines(&self.data_dir.path(kind))?;
        Ok(Document::parse(&lines, &self.names, self.today))
    }
}
