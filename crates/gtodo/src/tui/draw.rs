//! 画面の描画。
//!
//! 色は飾りに留め、意味は記号・太字・下線でも分かるようにする (`NO_COLOR` や色の少ない端末のため)。

use chrono::NaiveDate;
use gtodo_core::sections::SectionKey;
use gtodo_core::task::{Status, Task};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, Level, Mode, Row, TABS, Tab};
use super::keymap::{Action, Key, sequence_label};

/// 選んでいる行の左端の印。
pub const GUTTER: &str = "▌";
/// メッセージ欄の最大の行数。
const MAX_MESSAGE_LINES: usize = 6;
/// 本文を切り詰めるときも残す幅。これより狭いときはタグの方を切る。
const MIN_CONTENT_WIDTH: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub color: bool,
}

impl Theme {
    /// `NO_COLOR` が空でない値で設定されていれば色を使わない (<https://no-color.org>)。
    pub fn from_env() -> Theme {
        Theme {
            color: std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
        }
    }

    fn fg(&self, color: Color) -> Style {
        if self.color {
            Style::new().fg(color)
        } else {
            Style::new()
        }
    }

    fn accent(&self) -> Style {
        self.fg(Color::Cyan)
    }

    fn dim(&self) -> Style {
        Style::new().add_modifier(Modifier::DIM)
    }

    fn key(&self) -> Style {
        self.accent().add_modifier(Modifier::BOLD)
    }
}

pub fn draw(frame: &mut Frame, app: &mut App, theme: Theme) {
    let area = frame.area();
    let messages = message_lines(app, theme, area.width as usize);
    let footer = footer_lines(app, theme, area.width as usize);
    let [tabs_area, list_area, message_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(messages.len() as u16),
        Constraint::Length(footer.len() as u16),
    ])
    .areas(area);

    frame.render_widget(Paragraph::new(tab_line(app, theme)), tabs_area);
    draw_list(frame, app, theme, list_area);
    frame.render_widget(Paragraph::new(messages), message_area);
    frame.render_widget(Paragraph::new(footer), footer_area);

    if let Mode::WaitInput { input, .. } = &app.mode {
        let x = footer_area.x + (WAIT_PROMPT.width() + input.cursor_width()) as u16;
        let y = footer_area.y + footer_area.height.saturating_sub(1);
        frame.set_cursor_position((x.min(area.right().saturating_sub(1)), y));
    }
    if app.mode == Mode::Help {
        draw_help(frame, app, theme, area);
    }
}

fn tab_line(app: &App, theme: Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, tab) in TABS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("│", theme.dim()));
        }
        let text = format!(" {} {} {} ", i + 1, tab.label(&app.ws.names), app.tabs[i].rows.len());
        let style = if i == app.current {
            theme.accent().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::new()
        };
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

fn draw_list(frame: &mut Frame, app: &mut App, theme: Theme, area: Rect) {
    let block = Block::new().borders(Borders::TOP).border_style(theme.dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let height = inner.height as usize;
    app.page_height = height;
    let tab = app.tab();
    let today = app.today();
    let view = app.view_mut();
    if view.rows.is_empty() {
        frame.render_widget(Paragraph::new(Span::styled("  タスクはありません", theme.dim())), inner);
        return;
    }
    // 選んでいる行が見える範囲に表示位置を合わせる。
    if view.cursor < view.offset {
        view.offset = view.cursor;
    } else if height > 0 && view.cursor >= view.offset + height {
        view.offset = view.cursor + 1 - height;
    }
    view.offset = view.offset.min(view.rows.len().saturating_sub(height.max(1)));

    let lines: Vec<Line> = view
        .rows
        .iter()
        .enumerate()
        .skip(view.offset)
        .take(height)
        .map(|(i, row)| row_line(row, i == view.cursor, tab, today, inner.width as usize, theme))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// 一覧の 1 行。本文が入りきらなければ、タグ・期日を残して本文を `…` で切る。
pub fn row_line(row: &Row, selected: bool, tab: Tab, today: NaiveDate, width: usize, theme: Theme) -> Line<'static> {
    let task = &row.task;
    let done = task.status == Status::Done;
    let base = if done { theme.dim() } else { Style::new() };

    let mut head = vec![
        if selected {
            Span::styled(GUTTER, theme.accent())
        } else {
            Span::raw(" ")
        },
        Span::raw(" "),
        Span::styled(if done { "[x] " } else { "[ ] " }, base),
    ];
    if let Some(p) = task.priority {
        head.push(Span::styled(format!("({p}) "), base.add_modifier(Modifier::BOLD)));
    }
    let tail = tail_spans(task, tab, today, base, theme);

    let used: usize = head.iter().chain(&tail).map(Span::width).sum();
    let room = width.saturating_sub(used).max(MIN_CONTENT_WIDTH);
    let content = truncate(&task.content, room);
    let content_style = if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    };

    let mut spans = head;
    spans.push(Span::styled(content, content_style));
    spans.extend(tail);
    Line::from(spans)
}

/// 本文の後ろに出すタグと日付。
fn tail_spans(task: &Task, tab: Tab, today: NaiveDate, base: Style, theme: Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let value = |v: &Option<String>| v.as_deref().filter(|v| !v.is_empty()).map(str::to_string);
    if let Some(p) = value(&task.project) {
        spans.push(Span::styled(format!(" +{p}"), base.patch(theme.fg(Color::Magenta))));
    }
    if let Some(c) = value(&task.context) {
        spans.push(Span::styled(
            format!(" @{}", c.trim_start_matches('@')),
            base.patch(theme.fg(Color::Green)),
        ));
    }
    match tab {
        Tab::Done => {
            if let Some(d) = value(&task.completed_at) {
                spans.push(Span::styled(format!("  {d}"), theme.dim()));
            }
        }
        Tab::Cancelled => {
            if let Some(d) = value(&task.cancelled) {
                spans.push(Span::styled(format!("  {d}"), theme.dim()));
            }
        }
        _ => {
            if let Some(due) = value(&task.due) {
                let (text, urgency) = due_label(&due, today);
                let style = if task.status == Status::Done {
                    base
                } else {
                    match urgency {
                        Urgency::Overdue => theme.fg(Color::Red).add_modifier(Modifier::BOLD),
                        Urgency::Today => theme.fg(Color::Yellow).add_modifier(Modifier::BOLD),
                        Urgency::Later => Style::new(),
                    }
                };
                spans.push(Span::styled(format!("  {text}"), style));
            }
            if let Some(w) = value(&task.wait) {
                spans.push(Span::styled(format!("  wait:{w}"), base.patch(theme.fg(Color::Blue))));
            }
        }
    }
    spans
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Overdue,
    Today,
    Later,
}

/// 期日の表示 (Neovim 版のカンバンと同じ文言)。日付として読めなければそのまま出す。
pub fn due_label(due: &str, today: NaiveDate) -> (String, Urgency) {
    let Ok(date) = NaiveDate::parse_from_str(due, "%Y-%m-%d") else {
        return (format!("due:{due}"), Urgency::Later);
    };
    let days = (date - today).num_days();
    match days {
        d if d < 0 => (format!("due:{due} ({}日超過)", -d), Urgency::Overdue),
        0 => (format!("due:{due} (今日)"), Urgency::Today),
        1 => (format!("due:{due} (明日)"), Urgency::Later),
        d => (format!("due:{due} ({d}日後)"), Urgency::Later),
    }
}

/// 表示幅 `width` に収まるよう切り詰める (超えた分は `…`)。
pub fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

fn message_lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for m in &app.messages {
        let (prefix, style) = match m.level {
            Level::Info => ("", Style::new()),
            Level::Warning => ("warning: ", theme.fg(Color::Yellow)),
            Level::Error => ("error: ", theme.fg(Color::Red).add_modifier(Modifier::BOLD)),
        };
        for (i, text) in m.text.lines().enumerate() {
            let text = if i == 0 {
                format!("{prefix}{text}")
            } else {
                text.to_string()
            };
            lines.push(Line::styled(truncate(&text, width), style));
        }
    }
    if lines.len() > MAX_MESSAGE_LINES {
        let rest = lines.len() - (MAX_MESSAGE_LINES - 1);
        lines.truncate(MAX_MESSAGE_LINES - 1);
        lines.push(Line::styled(format!("… (+{rest})"), theme.dim()));
    }
    lines
}

/// Neovim 版の入力欄と同じ問いかけ。
const WAIT_PROMPT: &str = "Waiting for (empty to skip/remove): ";

fn footer_lines(app: &App, theme: Theme, width: usize) -> Vec<Line<'static>> {
    match &app.mode {
        Mode::WaitInput { content, input, .. } => vec![
            Line::styled(
                format!("→ {}: {content}", app.ws.names.name(SectionKey::Waiting)),
                theme.dim(),
            ),
            Line::from(vec![
                Span::styled(WAIT_PROMPT, theme.key()),
                Span::raw(input.text().to_string()),
            ]),
        ],
        Mode::ConfirmCancel { content, .. } => vec![
            Line::styled(format!("→ cancelled.md: {content}"), theme.dim()),
            Line::from(Span::styled("Cancel this task? [y/N]", theme.key())),
        ],
        Mode::Help => vec![Line::styled("何かキーを押すと閉じる", theme.dim())],
        Mode::Normal if !app.pending.is_empty() => {
            let mut hints: Vec<(String, String)> = app
                .keymap
                .continuations(&app.pending)
                .map(|(keys, action)| (sequence_label(keys), description(app, action)))
                .collect();
            hints.push(("Esc".to_string(), "取り消す".to_string()));
            vec![hint_line(&hints, theme)]
        }
        Mode::Normal => vec![hint_line(&fit_hints(normal_hints(app), width), theme)],
    }
}

fn hint_line(hints: &[(String, String)], theme: Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (key, desc)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key.clone(), theme.key()));
        spans.push(Span::raw(format!(" {desc}")));
    }
    Line::from(spans)
}

/// 選んでいるタスクで押したときに起きることを出す。
fn normal_hints(app: &App) -> Vec<(Action, String, String)> {
    let mut hints = Vec::new();
    let mut add = |action: Action, desc: String| {
        if let Some(key) = app.keymap.label(action) {
            hints.push((action, key, desc));
        }
    };
    let tab = app.tab();
    let names = &app.ws.names;
    if let Some(row) = app.view().selected().filter(|_| tab.writable()) {
        match row.task.status {
            Status::Open => {
                add(Action::ToggleDone, "完了にする".to_string());
                for (action, key) in [
                    (Action::MoveToday, SectionKey::Today),
                    (Action::MoveNext, SectionKey::Next),
                    (Action::MoveWaiting, SectionKey::Waiting),
                    (Action::MoveSomeday, SectionKey::Someday),
                ] {
                    if tab == Tab::Section(key) {
                        if key == SectionKey::Waiting {
                            add(action, "待ち先を変える".to_string());
                        }
                        continue;
                    }
                    add(action, format!("{} へ", names.name(key)));
                }
            }
            Status::Done => add(Action::ToggleDone, "未完了に戻す".to_string()),
        }
        add(Action::Cancel, "キャンセル".to_string());
    }
    if tab.writable() {
        add(Action::Sort, "並べ替え".to_string());
    }
    add(Action::Help, "ヘルプ".to_string());
    add(Action::Quit, "終了".to_string());
    hints
}

/// 幅に収まるまで、重要でない案内から外す。`?` (ヘルプ) は外さない (外したキーもヘルプで分かるように)。
fn fit_hints(mut hints: Vec<(Action, String, String)>, width: usize) -> Vec<(String, String)> {
    const DROP_ORDER: [Action; 8] = [
        Action::Quit,
        Action::Sort,
        Action::Cancel,
        Action::MoveSomeday,
        Action::MoveWaiting,
        Action::MoveNext,
        Action::MoveToday,
        Action::ToggleDone,
    ];
    let line_width = |hints: &[(Action, String, String)]| -> usize {
        let items: usize = hints.iter().map(|(_, k, d)| k.width() + 1 + d.width()).sum();
        items + 2 * hints.len().saturating_sub(1)
    };
    for action in DROP_ORDER {
        if line_width(&hints) <= width {
            break;
        }
        hints.retain(|(a, _, _)| *a != action);
    }
    hints.into_iter().map(|(_, k, d)| (k, d)).collect()
}

/// `?` のヘルプと、押しかけのキーの候補に出す説明。
fn description(app: &App, action: Action) -> String {
    let names = &app.ws.names;
    match action {
        Action::Down => "下へ".to_string(),
        Action::Up => "上へ".to_string(),
        Action::Top => "先頭へ".to_string(),
        Action::Bottom => "末尾へ".to_string(),
        Action::HalfPageDown => "半ページ下へ".to_string(),
        Action::HalfPageUp => "半ページ上へ".to_string(),
        Action::PageDown => "1 ページ下へ".to_string(),
        Action::PageUp => "1 ページ上へ".to_string(),
        Action::NextTab => "次のタブ".to_string(),
        Action::PrevTab => "前のタブ".to_string(),
        Action::GoTab(i) => format!("{} のタブ", TABS[i].label(names)),
        Action::MoveToday => format!("{} へ移す", names.name(SectionKey::Today)),
        Action::MoveNext => format!("{} へ移す", names.name(SectionKey::Next)),
        Action::MoveWaiting => format!(
            "{} へ移す (待ち先を入れる。{} にあれば待ち先を変える)",
            names.name(SectionKey::Waiting),
            names.name(SectionKey::Waiting)
        ),
        Action::MoveSomeday => format!("{} へ移す", names.name(SectionKey::Someday)),
        Action::ToggleDone => "完了と未完了を切り替える".to_string(),
        Action::Cancel => "キャンセルする (cancelled.md へ移す。確認あり)".to_string(),
        Action::Sort => "期日が来たタスクを移し、並べ替える".to_string(),
        Action::Help => "このヘルプ".to_string(),
        Action::Quit => "終了".to_string(),
    }
}

/// ヘルプの行 (キー, 説明)。同じ動作のキーは 1 行にまとめる。
///
/// タブの番号のキーが既定のまま (`1`〜`7`) なら 1 行にまとめる (24 行の端末にヘルプが収まるように)。
pub fn help_entries(app: &App) -> Vec<(String, String)> {
    let digits_only = (0..TABS.len()).all(|i| {
        let keys: Vec<&[Key]> = app.keymap.keys_for(Action::GoTab(i)).collect();
        keys.len() == 1 && keys[0] == [Key::char(char::from(b'1' + i as u8))]
    });
    let mut entries = Vec::new();
    for action in all_actions() {
        if digits_only && matches!(action, Action::GoTab(_)) {
            if action == Action::GoTab(0) {
                entries.push((format!("1〜{}", TABS.len()), "番号のタブへ".to_string()));
            }
            continue;
        }
        let keys: Vec<String> = app.keymap.keys_for(action).map(sequence_label).collect();
        if !keys.is_empty() {
            entries.push((keys.join(" / "), description(app, action)));
        }
    }
    entries
}

fn all_actions() -> Vec<Action> {
    let mut actions = vec![
        Action::Down,
        Action::Up,
        Action::Top,
        Action::Bottom,
        Action::HalfPageDown,
        Action::HalfPageUp,
        Action::PageDown,
        Action::PageUp,
        Action::NextTab,
        Action::PrevTab,
    ];
    actions.extend((0..TABS.len()).map(Action::GoTab));
    actions.extend([
        Action::ToggleDone,
        Action::MoveToday,
        Action::MoveNext,
        Action::MoveWaiting,
        Action::MoveSomeday,
        Action::Cancel,
        Action::Sort,
        Action::Help,
        Action::Quit,
    ]);
    actions
}

fn draw_help(frame: &mut Frame, app: &App, theme: Theme, area: Rect) {
    let entries = help_entries(app);
    let key_width = entries.iter().map(|(k, _)| k.width()).max().unwrap_or(0);
    let lines: Vec<Line> = entries
        .iter()
        .map(|(keys, desc)| {
            let pad = " ".repeat(key_width - keys.width());
            Line::from(vec![
                Span::raw(" "),
                Span::styled(keys.clone(), theme.key()),
                Span::raw(format!("{pad}  {desc}")),
            ])
        })
        .collect();
    let content_width = lines.iter().map(Line::width).max().unwrap_or(0) + 2;
    let width = (content_width as u16 + 2).min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" ヘルプ ").border_style(theme.accent())),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn due_labels_match_the_neovim_kanban() {
        let today = date("2026-11-16");
        assert_eq!(
            due_label("2026-11-14", today),
            ("due:2026-11-14 (2日超過)".into(), Urgency::Overdue)
        );
        assert_eq!(
            due_label("2026-11-16", today),
            ("due:2026-11-16 (今日)".into(), Urgency::Today)
        );
        assert_eq!(
            due_label("2026-11-17", today),
            ("due:2026-11-17 (明日)".into(), Urgency::Later)
        );
        assert_eq!(
            due_label("2026-11-20", today),
            ("due:2026-11-20 (4日後)".into(), Urgency::Later)
        );
        assert_eq!(due_label("someday", today), ("due:someday".into(), Urgency::Later));
    }

    #[test]
    fn truncate_counts_display_width() {
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "ab…");
        assert_eq!(truncate("日本語", 5), "日本…");
        assert_eq!(truncate("日本語", 4), "日…");
    }
}
