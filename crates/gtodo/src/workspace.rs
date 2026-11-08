//! `data_dir` とその設定を読み込んだ作業環境。

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use chrono::NaiveDate;
use gtodo_core::document::Document;
use gtodo_core::sections::{SectionKey, SectionNameError, SectionNames};
use gtodo_core::store::{self, DataDir, FileKind};
use gtodo_core::{settings, state};

pub struct Workspace {
    pub data_dir: DataDir,
    pub names: SectionNames,
    pub today: NaiveDate,
    pub auto_move_inbox_to_today: bool,
    /// Neovim 版が最後に起動したときの名前を現在の名前にできず、既定の名前で読んだ理由。
    pub unusable_last_names: Option<SectionNameError>,
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
        let (names, unusable_last_names) = section_names(settings.sections, &data_dir.state_file(), warn);
        Ok(Workspace {
            data_dir,
            names,
            today,
            auto_move_inbox_to_today: settings.auto_move_inbox_to_today,
            unusable_last_names,
        })
    }

    pub fn load(&self, kind: FileKind) -> Result<Document> {
        let lines = store::read_file(&self.data_dir.path(kind))?.lines;
        Ok(Document::parse(&lines, &self.names, self.today))
    }
}

/// 現在のセクション名に、Neovim 版が最後に起動したときに使っていた名前を別名として加える。
/// `gtodo.json` でセクション名を設定していなければ、Neovim 版の名前を現在の名前にする
/// ([`SectionNames::from_last`])。
///
/// `.state.json` を読めなくても一覧は表示できるため止めないが、黙って無視すると
/// 壊れていることに気づけないので警告する。
///
/// Neovim 版の名前を現在の名前にできなければ既定の名前で読み、その理由も返す。このとき
/// Neovim 版の名前は別名にも加えない。使えない名前は他のセクションの名前と重なっており、
/// 加えると 2 つのセクションが 1 つに合流して読まれるため。
fn section_names(
    configured: Option<SectionNames>,
    state_file: &Path,
    warn: &mut impl FnMut(String),
) -> (SectionNames, Option<SectionNameError>) {
    let last = match state::State::read(state_file) {
        Ok(state) => state.last_sections(),
        Err(e) => {
            warn(format!("{e}; ignoring the section names Neovim used last"));
            Default::default()
        }
    };
    let current = match configured {
        Some(names) => names,
        None => match SectionNames::from_last(&last) {
            Ok(names) => names,
            Err(e) => {
                warn(format!(
                    "the section names Neovim used last are invalid ({e}); reading with the default names"
                ));
                return (SectionNames::default(), Some(e));
            }
        },
    };
    let names = SectionKey::ALL
        .into_iter()
        .zip(last)
        .fold(current, |names, (key, name)| names.with_last(key, name));
    (names, None)
}
