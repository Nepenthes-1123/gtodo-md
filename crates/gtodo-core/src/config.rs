//! ユーザー設定 (`config.json`) と `data_dir` の場所の解決。Neovim 版と共有する規則。
//!
//! `data_dir` の優先順:
//! 1. 呼び出し元の明示的な指定 (`--data-dir`)
//! 2. 環境変数 `GTODO_DATA_DIR`
//! 3. `config.json` の `data_dir` (絶対パスか `~` 始まりに限る)
//! 4. `$XDG_DATA_HOME/gtodo-md` が存在すればそこ
//! 5. Neovim 版の旧い既定 (`stdpath("data")/gtodo-md`) が存在すればそこ
//! 6. それ以外は `$XDG_DATA_HOME/gtodo-md`
//!
//! `config.json` は環境変数 `GTODO_CONFIG` があればそのパス、無ければ `$XDG_CONFIG_HOME/gtodo-md/config.json`。
//! XDG の環境変数が無いときの既定は、Unix (macOS を含む) が `~/.config`・`~/.local/share`、
//! Windows が `%APPDATA%`・`%LOCALAPPDATA%`。

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

const APP_DIR: &str = "gtodo-md";

/// 環境変数の読み取り。テストで差し替えられるよう引数で受け取る。
pub trait Env {
    /// 値が空の変数は未設定として扱う。
    fn var(&self, key: &str) -> Option<OsString>;
}

pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn var(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key).filter(|v| !v.is_empty())
    }
}

/// 既定の場所の決め方。テストで Windows の規則も検証できるよう、`cfg!` ではなく値で受け取る。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Unix,
    Windows,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Unix
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid config file {}: {reason}", path.display())]
    Invalid { path: PathBuf, reason: String },
    #[error("cannot expand '~' in {}: home directory is unknown", .0.display())]
    UnknownHome(PathBuf),
    #[error("cannot determine the default data directory: home directory is unknown")]
    NoDefaultDataDir,
}

/// `data_dir` を決める。`warn` には、処理を続けられるが利用者に知らせるべき問題が渡される。
pub fn resolve_data_dir(
    explicit: Option<PathBuf>,
    env: &impl Env,
    platform: Platform,
    warn: &mut impl FnMut(String),
) -> Result<PathBuf, ConfigError> {
    if let Some(dir) = explicit {
        return Ok(dir);
    }
    if let Some(dir) = env.var("GTODO_DATA_DIR") {
        return expand_home(PathBuf::from(dir), env, platform);
    }
    if let Some((path, explicit)) = config_path(env, platform) {
        if let Some(dir) = read_config_data_dir(&path, explicit, env, platform, warn)? {
            return Ok(dir);
        }
    }
    default_data_dir(env, platform, warn)
}

/// `config.json` のパスと、明示的に指定されたものか。
fn config_path(env: &impl Env, platform: Platform) -> Option<(PathBuf, bool)> {
    if let Some(path) = env.var("GTODO_CONFIG") {
        return Some((PathBuf::from(path), true));
    }
    let base = absolute_var(env, "XDG_CONFIG_HOME").or_else(|| match platform {
        Platform::Unix => home_dir(env, platform).map(|h| h.join(".config")),
        Platform::Windows => env
            .var("APPDATA")
            .map(PathBuf::from)
            .or_else(|| home_dir(env, platform).map(|h| h.join("AppData").join("Roaming"))),
    })?;
    Some((base.join(APP_DIR).join("config.json"), false))
}

fn read_config_data_dir(
    path: &Path,
    explicit: bool,
    env: &impl Env,
    platform: Platform,
    warn: &mut impl FnMut(String),
) -> Result<Option<PathBuf>, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        // 既定の場所に無いのは普通のこと。明示的に指定されたファイルが無いのは指定の誤り。
        Err(e) if e.kind() == io::ErrorKind::NotFound && !explicit => return Ok(None),
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let invalid = |reason: String| ConfigError::Invalid {
        path: path.to_path_buf(),
        reason,
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?;
    let Value::Object(top) = value else {
        return Err(invalid("top level is not an object".into()));
    };
    let mut data_dir = None;
    for (key, value) in top {
        match (key.as_str(), value) {
            ("data_dir", Value::String(dir)) => data_dir = Some(dir),
            ("data_dir", _) => return Err(invalid("\"data_dir\" is not a string".into())),
            // バージョンの違うツール同士で同じファイルを読むため、知らない項目では止めない。
            _ => warn(format!("{}: unknown key {key:?} is ignored", path.display())),
        }
    }
    let Some(dir) = data_dir else {
        return Ok(None);
    };
    let expanded = expand_home(PathBuf::from(&dir), env, platform)?;
    // 相対パスの XDG_* を無視するのと同じく、実行したディレクトリで場所が変わる設定は受け付けない。
    if !expanded.is_absolute() {
        return Err(invalid(format!(
            "\"data_dir\" must be an absolute path or start with '~', got {dir:?}"
        )));
    }
    Ok(Some(expanded))
}

/// 新しい既定があればそれを、無くて Neovim 版の以前の既定があればそれを使う。どちらも無ければ新しい既定。
///
/// Neovim 版も同じ順で決める。以前の既定にデータがある利用者は、移さずに使い続けられる。
/// 新しい既定が作られるのは、どちらも無いとき (Neovim 版の `setup()` が作る) だけ。
/// 両方あると以前の既定のデータが使われなくなり、タスクが消えたように見えるため警告する。
/// 警告は以前の既定を消すようには促さない。そちらにしかデータが無いこともあるため。
fn default_data_dir(env: &impl Env, platform: Platform, warn: &mut impl FnMut(String)) -> Result<PathBuf, ConfigError> {
    let new = data_home(env, platform).map(|d| d.join(APP_DIR));
    let legacy = neovim_legacy_data_dirs(env, platform)
        .into_iter()
        .find(|dir| dir.is_dir());
    if let Some(new) = new.as_ref().filter(|dir| dir.is_dir()) {
        if let Some(legacy) = legacy.filter(|dir| !same_dir(dir, new)) {
            warn(format!(
                "both {} and the old default {} exist; using {}. Tasks in {} are not used: \
                 move them to {}, or set data_dir to keep using it",
                new.display(),
                legacy.display(),
                new.display(),
                legacy.display(),
                new.display()
            ));
        }
        return Ok(new.clone());
    }
    if let Some(legacy) = legacy {
        return Ok(legacy);
    }
    new.ok_or(ConfigError::NoDefaultDataDir)
}

/// 同じディレクトリか。新しい既定を以前の既定へのシンボリックリンクにして共有している場合も同じとみなす。
///
/// 文字列として比べると、リンクで同じ場所を指していても「両方ある」と警告し、実体である
/// 以前の既定のデータを手放すよう促すことになる。
fn same_dir(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn data_home(env: &impl Env, platform: Platform) -> Option<PathBuf> {
    absolute_var(env, "XDG_DATA_HOME").or_else(|| match platform {
        Platform::Unix => home_dir(env, platform).map(|h| h.join(".local").join("share")),
        Platform::Windows => local_app_data(env, platform),
    })
}

/// Neovim 版の以前の既定 (`stdpath("data") .. "/gtodo-md"`) の候補。探す順に返す。
///
/// `NVIM_APPNAME` のもの (設定されていれば) と、既定の名前 (`nvim`) のもの。後者も探すのは、
/// 別の名前で起動した Neovim では、そのプロファイルの以前の既定が無いために新しい既定を作って
/// しまうため。作られると、以後は通常の Neovim も CLI も新しい既定 (空) を使い、`nvim` の
/// 以前の既定にあるタスクが消えたように見える。Neovim 版も同じ候補を同じ順で探す。
///
/// Neovim は相対パスの `XDG_DATA_HOME` もそのまま使うが、ここでは XDG の仕様どおり無視する
/// (カレントディレクトリによって場所が変わり、意図した設定とは考えにくいため)。
fn neovim_legacy_data_dirs(env: &impl Env, platform: Platform) -> Vec<PathBuf> {
    let base = match platform {
        Platform::Unix => absolute_var(env, "XDG_DATA_HOME")
            .or_else(|| home_dir(env, platform).map(|h| h.join(".local").join("share"))),
        Platform::Windows => absolute_var(env, "XDG_DATA_HOME").or_else(|| local_app_data(env, platform)),
    };
    let Some(base) = base else {
        return Vec::new();
    };
    let dir = |app: &OsString| {
        let mut app = app.clone();
        if platform == Platform::Windows {
            app.push("-data");
        }
        base.join(app).join(APP_DIR)
    };
    let default_app = OsString::from("nvim");
    let mut dirs = Vec::new();
    if let Some(app) = env.var("NVIM_APPNAME").filter(|app| *app != default_app) {
        dirs.push(dir(&app));
    }
    dirs.push(dir(&default_app));
    dirs
}

fn local_app_data(env: &impl Env, platform: Platform) -> Option<PathBuf> {
    env.var("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| home_dir(env, platform).map(|h| h.join("AppData").join("Local")))
}

/// XDG の仕様どおり、相対パスの値は無効として扱う。
fn absolute_var(env: &impl Env, key: &str) -> Option<PathBuf> {
    env.var(key).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn home_dir(env: &impl Env, platform: Platform) -> Option<PathBuf> {
    let key = match platform {
        Platform::Unix => "HOME",
        Platform::Windows => "USERPROFILE",
    };
    env.var(key).map(PathBuf::from)
}

/// 先頭の `~` をホームディレクトリへ展開する (設定ファイルや環境変数の値はシェルが展開しないため)。
fn expand_home(path: PathBuf, env: &impl Env, platform: Platform) -> Result<PathBuf, ConfigError> {
    let Ok(rest) = path.strip_prefix("~") else {
        return Ok(path);
    };
    let home = home_dir(env, platform).ok_or_else(|| ConfigError::UnknownHome(path.clone()))?;
    Ok(home.join(rest))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct FakeEnv(HashMap<&'static str, String>);

    impl Env for FakeEnv {
        fn var(&self, key: &str) -> Option<OsString> {
            self.0.get(key).filter(|v| !v.is_empty()).map(OsString::from)
        }
    }

    fn env(pairs: &[(&'static str, &str)]) -> FakeEnv {
        FakeEnv(pairs.iter().map(|(k, v)| (*k, v.to_string())).collect())
    }

    /// テスト用のホームディレクトリ。存在確認があるため実在のディレクトリを使う。
    struct Home(tempfile::TempDir);

    impl Home {
        fn new() -> Home {
            Home(tempfile::tempdir().unwrap())
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.0.path().join(rel)
        }

        fn str(&self) -> &str {
            self.0.path().to_str().unwrap()
        }

        fn mkdir(&self, rel: &str) -> PathBuf {
            let p = self.path(rel);
            fs::create_dir_all(&p).unwrap();
            p
        }

        fn write(&self, rel: &str, content: &str) -> PathBuf {
            let p = self.path(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, content).unwrap();
            p
        }
    }

    /// `config.json` の `data_dir` には、一時ディレクトリの下のパスを書く。
    /// `/from/config` のような値は Windows ではドライブ名が無いため絶対パスにならず、エラーになる。
    fn config_with_data_dir(dir: &Path) -> String {
        serde_json::json!({ "data_dir": dir.to_str().unwrap() }).to_string()
    }

    fn resolve(e: &FakeEnv, platform: Platform) -> Result<PathBuf, ConfigError> {
        resolve_data_dir(None, e, platform, &mut |w| panic!("unexpected warning: {w}"))
    }

    #[test]
    fn data_dir_precedence_is_flag_then_env_then_config_then_default() {
        let home = Home::new();
        let from_config = home.path("from-config");
        home.write(".config/gtodo-md/config.json", &config_with_data_dir(&from_config));
        let base = env(&[("HOME", home.str())]);
        let with_env = env(&[("HOME", home.str()), ("GTODO_DATA_DIR", "/from/env")]);

        let flag = resolve_data_dir(Some("/from/flag".into()), &with_env, Platform::Unix, &mut |_| {}).unwrap();
        assert_eq!(flag, Path::new("/from/flag"));
        assert_eq!(resolve(&with_env, Platform::Unix).unwrap(), Path::new("/from/env"));
        assert_eq!(resolve(&base, Platform::Unix).unwrap(), from_config);

        fs::remove_file(home.path(".config/gtodo-md/config.json")).unwrap();
        assert_eq!(
            resolve(&base, Platform::Unix).unwrap(),
            home.path(".local/share/gtodo-md")
        );
    }

    #[test]
    fn neovim_old_default_is_used_only_while_the_new_one_does_not_exist() {
        let home = Home::new();
        let e = env(&[("HOME", home.str())]);
        let legacy = home.mkdir(".local/share/nvim/gtodo-md");
        assert_eq!(resolve(&e, Platform::Unix).unwrap(), legacy);
        assert!(!home.path(".local/share/gtodo-md").exists());

        let new = home.mkdir(".local/share/gtodo-md");
        fs::remove_dir(&legacy).unwrap();
        assert_eq!(resolve(&e, Platform::Unix).unwrap(), new);
    }

    #[test]
    fn another_neovim_profile_uses_the_default_profiles_old_directory_instead_of_creating_the_new_one() {
        let home = Home::new();
        let nvim_legacy = home.mkdir(".local/share/nvim/gtodo-md");
        let e = env(&[("HOME", home.str()), ("NVIM_APPNAME", "lazyvim")]);
        assert_eq!(resolve(&e, Platform::Unix).unwrap(), nvim_legacy);

        let own_legacy = home.mkdir(".local/share/lazyvim/gtodo-md");
        assert_eq!(
            resolve(&e, Platform::Unix).unwrap(),
            own_legacy,
            "the profile's own old directory comes first"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_new_default_linked_to_the_old_one_is_not_warned_about() {
        let home = Home::new();
        let legacy = home.mkdir(".local/share/nvim/gtodo-md");
        std::os::unix::fs::symlink(&legacy, home.path(".local/share/gtodo-md")).unwrap();
        let e = env(&[("HOME", home.str())]);
        assert_eq!(
            fs::canonicalize(resolve(&e, Platform::Unix).unwrap()).unwrap(),
            fs::canonicalize(&legacy).unwrap()
        );
    }

    #[test]
    fn new_default_wins_and_warns_when_both_exist() {
        let home = Home::new();
        let e = env(&[("HOME", home.str())]);
        let new = home.mkdir(".local/share/gtodo-md");
        home.mkdir(".local/share/nvim/gtodo-md");
        let mut warnings = Vec::new();
        let dir = resolve_data_dir(None, &e, Platform::Unix, &mut |w| warnings.push(w)).unwrap();
        assert_eq!(dir, new);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        // 警告に出るのは解決したパス。`home.mkdir` のパスは区切りに `/` を含むため、
        // Windows では文字列としては一致しない。部品ごとに組み立てたパスで比べる。
        let under_home = |parts: &[&str]| parts.iter().fold(home.0.path().to_path_buf(), |p, s| p.join(s));
        for expected in [
            under_home(&[".local", "share", "gtodo-md"]),
            under_home(&[".local", "share", "nvim", "gtodo-md"]),
        ] {
            let expected = expected.display().to_string();
            assert!(warnings[0].contains(&expected), "{expected} in {warnings:?}");
        }
    }

    #[test]
    fn legacy_dir_follows_nvim_appname_and_xdg_data_home() {
        let home = Home::new();
        let xdg = home.mkdir("xdg");
        let legacy = home.mkdir("xdg/lazyvim/gtodo-md");
        let e = env(&[
            ("HOME", home.str()),
            ("XDG_DATA_HOME", xdg.to_str().unwrap()),
            ("NVIM_APPNAME", "lazyvim"),
        ]);
        assert_eq!(resolve(&e, Platform::Unix).unwrap(), legacy);
    }

    #[test]
    fn xdg_variables_take_precedence_on_every_platform() {
        let home = Home::new();
        let xdg_data = home.mkdir("xdg-data");
        home.write("xdg-config/gtodo-md/config.json", "{}");
        let e = env(&[
            ("USERPROFILE", home.str()),
            ("LOCALAPPDATA", "/unused/local"),
            ("APPDATA", "/unused/roaming"),
            ("XDG_DATA_HOME", xdg_data.to_str().unwrap()),
            ("XDG_CONFIG_HOME", home.path("xdg-config").to_str().unwrap()),
        ]);
        assert_eq!(resolve(&e, Platform::Windows).unwrap(), xdg_data.join("gtodo-md"));

        let legacy = home.mkdir("xdg-data/nvim-data/gtodo-md");
        assert_eq!(resolve(&e, Platform::Windows).unwrap(), legacy);
    }

    #[test]
    fn windows_uses_appdata_for_config_and_localappdata_for_data() {
        let home = Home::new();
        let local = home.mkdir("Local");
        home.write("Roaming/gtodo-md/config.json", r#"{"data_dir":"~/from-config"}"#);
        let e = env(&[
            ("USERPROFILE", home.str()),
            ("APPDATA", home.path("Roaming").to_str().unwrap()),
            ("LOCALAPPDATA", local.to_str().unwrap()),
        ]);
        assert_eq!(resolve(&e, Platform::Windows).unwrap(), home.path("from-config"));

        fs::remove_file(home.path("Roaming/gtodo-md/config.json")).unwrap();
        assert_eq!(resolve(&e, Platform::Windows).unwrap(), local.join("gtodo-md"));
        let legacy = home.mkdir("Local/nvim-data/gtodo-md");
        assert_eq!(resolve(&e, Platform::Windows).unwrap(), legacy);
    }

    #[test]
    fn windows_without_appdata_variables_uses_the_profile_directory() {
        let home = Home::new();
        let from_config = home.path("from-config");
        home.write(
            "AppData/Roaming/gtodo-md/config.json",
            &config_with_data_dir(&from_config),
        );
        let e = env(&[("USERPROFILE", home.str())]);
        assert_eq!(resolve(&e, Platform::Windows).unwrap(), from_config);
        fs::remove_file(home.path("AppData/Roaming/gtodo-md/config.json")).unwrap();
        assert_eq!(
            resolve(&e, Platform::Windows).unwrap(),
            home.path("AppData/Local/gtodo-md")
        );
    }

    #[test]
    fn relative_xdg_variables_are_ignored() {
        let home = Home::new();
        let e = env(&[
            ("HOME", home.str()),
            ("XDG_DATA_HOME", "relative/data"),
            ("XDG_CONFIG_HOME", "relative/config"),
        ]);
        assert_eq!(resolve(&e, Platform::Unix).unwrap(), home.path(".local/share/gtodo-md"));
    }

    #[test]
    fn tilde_in_config_and_env_expands_to_home() {
        let home = Home::new();
        let cfg = home.write("c.json", r#"{"data_dir":"~/todo"}"#);
        let e = env(&[("HOME", home.str()), ("GTODO_CONFIG", cfg.to_str().unwrap())]);
        assert_eq!(resolve(&e, Platform::Unix).unwrap(), home.path("todo"));
        let e = env(&[("HOME", home.str()), ("GTODO_DATA_DIR", "~/env-todo")]);
        assert_eq!(resolve(&e, Platform::Unix).unwrap(), home.path("env-todo"));
    }

    #[test]
    fn missing_default_config_is_fine_but_missing_explicit_one_is_an_error() {
        let home = Home::new();
        assert!(resolve(&env(&[("HOME", home.str())]), Platform::Unix).is_ok());
        let missing = home.path("none.json");
        let e = env(&[("HOME", home.str()), ("GTODO_CONFIG", missing.to_str().unwrap())]);
        assert!(matches!(resolve(&e, Platform::Unix), Err(ConfigError::Io { .. })));
    }

    #[test]
    fn unknown_keys_are_warned_and_malformed_values_are_errors() {
        let home = Home::new();
        let ok = home.path("ok");
        let config = serde_json::json!({ "data_dri": "/typo", "data_dir": ok.to_str().unwrap() });
        let cfg = home.write("c.json", &config.to_string());
        let e = env(&[("HOME", home.str()), ("GTODO_CONFIG", cfg.to_str().unwrap())]);
        let mut warnings = Vec::new();
        let dir = resolve_data_dir(None, &e, Platform::Unix, &mut |w| warnings.push(w)).unwrap();
        assert_eq!(dir, ok);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("\"data_dri\""), "{warnings:?}");

        for content in [
            "",
            "{broken",
            "[]",
            r#"{"data_dir":1}"#,
            r#"{"data_dir":""}"#,
            r#"{"data_dir":"relative/dir"}"#,
        ] {
            home.write("c.json", content);
            assert!(
                matches!(resolve(&e, Platform::Unix), Err(ConfigError::Invalid { .. })),
                "{content:?}"
            );
        }
    }

    #[test]
    fn config_is_not_read_when_a_higher_priority_source_is_given() {
        let home = Home::new();
        let cfg = home.write("c.json", "{broken");
        let e = env(&[
            ("HOME", home.str()),
            ("GTODO_CONFIG", cfg.to_str().unwrap()),
            ("GTODO_DATA_DIR", "/from/env"),
        ]);
        assert_eq!(resolve(&e, Platform::Unix).unwrap(), Path::new("/from/env"));
    }

    #[test]
    fn without_any_home_the_default_cannot_be_determined() {
        assert!(matches!(
            resolve(&env(&[]), Platform::Unix),
            Err(ConfigError::NoDefaultDataDir)
        ));
    }
}
