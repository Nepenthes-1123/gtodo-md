//! 利用者のエディタ (`$VISUAL`・`$EDITOR`) でファイルを開くコマンドを作る。

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use gtodo_core::config::{Env, Platform};

/// 利用者のエディタ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    /// 環境変数に書かれたままのコマンド。引数を含むことがある (`code --wait` など)。
    command: String,
    platform: Platform,
}

impl Editor {
    /// `VISUAL` → `EDITOR` → `vi` (Windows は `notepad`) の順に決める (git などと同じ順)。
    /// 値が空の変数は無いものとして扱う。
    pub fn from_env(env: &impl Env, platform: Platform) -> Editor {
        let command = ["VISUAL", "EDITOR"]
            .into_iter()
            .filter_map(|key| env.var(key))
            .map(|v| v.to_string_lossy().trim().to_string())
            .find(|v| !v.is_empty())
            .unwrap_or_else(|| {
                match platform {
                    Platform::Unix => "vi",
                    Platform::Windows => "notepad",
                }
                .to_string()
            });
        Editor { command, platform }
    }

    /// 知らせに出す名前 (書かれたままのコマンド)。
    pub fn name(&self) -> &str {
        &self.command
    }

    /// `file` を開くコマンド。`line` (1 始まり) は、行を指定する書き方を知っているエディタにだけ渡す
    /// (知らないエディタに `+12` を渡すと、そういう名前のファイルとして開かれうるため)。
    pub fn command(&self, file: &Path, line: Option<usize>) -> Command {
        let words = split_words(&self.command);
        let args = file_args(words.first().map_or("", String::as_str), file, line);
        match self.platform {
            Platform::Unix => {
                // git と同じく sh に渡す。引数やクォート、`~` を含む値も、シェルで書いたとおりに使える。
                let mut cmd = Command::new("sh");
                cmd.arg("-c")
                    .arg(format!("{} \"$@\"", self.command))
                    .arg(&self.command)
                    .args(args);
                cmd
            }
            Platform::Windows => {
                // cmd.exe を通すと、ファイルのパスの `&` などが解釈されるため、自分で分けて直接起動する。
                let mut cmd = Command::new(&words[0]);
                cmd.args(&words[1..]).args(args);
                cmd
            }
        }
    }
}

/// エディタに渡すファイルの引数。行の指定の書き方はエディタごとに違う。
fn file_args(program: &str, file: &Path, line: Option<usize>) -> Vec<OsString> {
    // ホストの OS に関わらず `/` と `\` の両方で区切る (`Path` は Unix では `\` で区切らない)。
    // 拡張子 (`.exe` など) は除く。
    let base = program.rsplit(['/', '\\']).next().unwrap_or_default();
    let name = base.split('.').next().unwrap_or_default().to_ascii_lowercase();
    match (name.as_str(), line) {
        (
            "vi" | "vim" | "nvim" | "nano" | "emacs" | "emacsclient" | "micro" | "kak" | "mg" | "joe" | "ne",
            Some(line),
        ) => {
            vec![format!("+{line}").into(), file.into()]
        }
        ("hx" | "helix", Some(line)) => {
            let mut arg = OsString::from(file);
            arg.push(format!(":{line}"));
            vec![arg]
        }
        _ => vec![file.into()],
    }
}

/// 空白で区切る。`"` で囲んだ部分は空白を含めて 1 つにする (`"C:\Program Files\..\x.exe" -n`)。
/// `\` はそのまま (Windows のパスの区切り)。空でない限り、最低 1 つの語を返す。
fn split_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started || words.is_empty() {
        words.push(word);
    }
    words
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    struct FakeEnv(HashMap<&'static str, &'static str>);

    impl Env for FakeEnv {
        fn var(&self, key: &str) -> Option<OsString> {
            self.0.get(key).filter(|v| !v.is_empty()).map(OsString::from)
        }
    }

    fn editor(vars: &[(&'static str, &'static str)], platform: Platform) -> Editor {
        Editor::from_env(&FakeEnv(vars.iter().copied().collect()), platform)
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn visual_comes_before_editor_and_vi_is_the_last_resort() {
        let both = [("VISUAL", "nvim"), ("EDITOR", "nano")];
        assert_eq!(editor(&both, Platform::Unix).name(), "nvim");
        assert_eq!(editor(&[("EDITOR", "nano")], Platform::Unix).name(), "nano");
        assert_eq!(
            editor(&[("VISUAL", " "), ("EDITOR", "nano")], Platform::Unix).name(),
            "nano"
        );
        assert_eq!(editor(&[], Platform::Unix).name(), "vi");
        assert_eq!(editor(&[], Platform::Windows).name(), "notepad");
    }

    #[test]
    fn unix_runs_the_editor_through_sh_like_git() {
        let file = Path::new("/tmp/x/todo.md");
        let cmd = editor(&[("EDITOR", "nvim -u NONE")], Platform::Unix).command(file, Some(12));
        assert_eq!(cmd.get_program(), "sh");
        assert_eq!(
            args(&cmd),
            ["-c", "nvim -u NONE \"$@\"", "nvim -u NONE", "+12", "/tmp/x/todo.md"]
        );
    }

    #[test]
    fn the_line_is_passed_only_to_editors_known_to_take_it() {
        let file = Path::new("/d/todo.md");
        let line = |program: &str| -> Vec<String> {
            file_args(program, file, Some(3))
                .into_iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        assert_eq!(line("vim"), ["+3", "/d/todo.md"]);
        assert_eq!(line("/usr/bin/nvim"), ["+3", "/d/todo.md"]);
        assert_eq!(line(r"C:\Program Files\Vim\vim91\VIM.EXE"), ["+3", "/d/todo.md"]);
        assert_eq!(line("hx"), ["/d/todo.md:3"]);
        assert_eq!(line("code"), ["/d/todo.md"]);
        assert_eq!(line("notepad"), ["/d/todo.md"]);
        assert_eq!(
            file_args("vim", file, None),
            [OsString::from("/d/todo.md")],
            "no line for an empty list"
        );
    }

    #[test]
    fn windows_splits_the_command_itself() {
        let e = editor(
            &[(
                "EDITOR",
                r#""C:\Program Files\Notepad++\notepad++.exe" -multiInst -nosession"#,
            )],
            Platform::Windows,
        );
        let cmd = e.command(Path::new(r"C:\t\todo.md"), Some(4));
        assert_eq!(cmd.get_program(), r"C:\Program Files\Notepad++\notepad++.exe");
        assert_eq!(args(&cmd), ["-multiInst", "-nosession", r"C:\t\todo.md"]);
        assert_eq!(split_words(r#"a "b c"  d"#), ["a", "b c", "d"]);
        assert_eq!(split_words(r#""""#), [""]);
    }
}
