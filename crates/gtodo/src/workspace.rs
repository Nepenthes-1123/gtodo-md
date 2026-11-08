//! `data_dir` とその設定を読み込んだ作業環境。

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use chrono::NaiveDate;
use gtodo_core::document::Document;
use gtodo_core::sections::{SectionKey, SectionNames};
use gtodo_core::store::{self, DataDir, FileKind};
use gtodo_core::{settings, state};

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
        let names = with_last_sections(settings.sections, &data_dir.state_file(), warn);
        Ok(Workspace { data_dir, names, today })
    }

    pub fn load(&self, kind: FileKind) -> Result<Document> {
        let lines = store::read_file(&self.data_dir.path(kind))?.lines;
        Ok(Document::parse(&lines, &self.names, self.today))
    }
}

/// Neovim 版が最後に起動したときに使っていたセクション名を別名として加える。
///
/// 読めなくても一覧は表示できるため止めないが、黙って無視すると `.state.json` が
/// 壊れていることに気づけないので警告する。
fn with_last_sections(names: SectionNames, state_file: &Path, warn: &mut impl FnMut(String)) -> SectionNames {
    match state::State::read(state_file).map(|s| s.last_sections()) {
        Ok(last) => SectionKey::ALL
            .into_iter()
            .zip(last)
            .fold(names, |names, (key, name)| names.with_last(key, name)),
        Err(e) => {
            warn(format!("{e}; ignoring the section names Neovim used last"));
            names
        }
    }
}
