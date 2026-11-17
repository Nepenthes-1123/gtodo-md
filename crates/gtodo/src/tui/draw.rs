//! 画面の描画。
//!
//! 色は飾りに留め、意味は記号・太字・下線でも分かるようにする (`NO_COLOR` や色の少ない端末のため)。

use chrono::NaiveDate;
use gtodo_core::ops::Destination;
use gtodo_core::sections::SectionKey;
use gtodo_core::task::{Status, Task};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, Level, Mode, Row, Screen, TABS, Tab};
use super::form::{DESTINATIONS, Field, Form, preview_line};
use super::keymap::{Action, Key, sequence_label};
use super::queue::{self, Heading};
use crate::render::{context_tag, project_tag};

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

    match app.screen {
        Screen::List => {
            frame.render_widget(Paragraph::new(tab_line(app, theme)), tabs_area);
            draw_list(frame, app, theme, list_area);
        }
        Screen::Queue => {
            let title = if app.queue.wait { "Queue (Wait)" } else { "Queue (Due) " };
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!(" {title} {}", app.today()),
                    theme.accent().add_modifier(Modifier::BOLD),
                )),
                tabs_area,
            );
            draw_queue(frame, app, theme, list_area);
        }
    }
    frame.render_widget(Paragraph::new(messages), message_area);
    frame.render_widget(Paragraph::new(footer), footer_area);

    if let Mode::WaitInput { input, .. } = &app.mode {
        let x = footer_area.x + (WAIT_PROMPT.width() + input.cursor_width()) as u16;
        let y = footer_area.y + footer_area.height.saturating_sub(1);
        frame.set_cursor_position((x.min(area.right().saturating_sub(1)), y));
    }
    if let Mode::Form(form) = &app.mode {
        let form_area = Rect {
            height: area.height.saturating_sub(footer_area.height),
            ..area
        };
        draw_form(frame, form, app, theme, form_area);
    }
    if app.mode == Mode::Help {
        // フッターは隠さない (スクロールの案内を出すため)。
        let help_area = Rect {
            height: area.height.saturating_sub(footer_area.height),
            ..area
        };
        draw_help(frame, app, theme, help_area);
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
    if let Some(p) = project_tag(task) {
        spans.push(Span::styled(format!(" {p}"), base.patch(theme.fg(Color::Magenta))));
    }
    if let Some(c) = context_tag(task) {
        spans.push(Span::styled(format!(" {c}"), base.patch(theme.fg(Color::Green))));
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
        Mode::WaitInput { row, input } => vec![
            Line::styled(
                format!("→ {}: {}", app.ws.names.name(SectionKey::Waiting), row.task.content),
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
        Mode::Help => {
            let scroll = [Action::Down, Action::Up]
                .iter()
                .filter_map(|a| app.keymap.label(*a))
                .collect::<Vec<_>>()
                .join("/");
            vec![hint_line(
                &[
                    (scroll, "スクロール".to_string()),
                    ("他のキー".to_string(), "閉じる".to_string()),
                ],
                theme,
            )]
        }
        Mode::Form(form) => {
            let mut hints = vec![
                ("Tab/↓".to_string(), "次の項目".to_string()),
                ("S-Tab/↑".to_string(), "前の項目".to_string()),
            ];
            match form.focus {
                Field::Destination => hints.push(("←/→".to_string(), "追加先を選ぶ".to_string())),
                Field::Project | Field::Context => hints.push(("Ctrl-n/p".to_string(), "候補".to_string())),
                _ => {}
            }
            hints.push(("Enter".to_string(), "確定".to_string()));
            hints.push(("Esc".to_string(), "取り消す".to_string()));
            vec![hint_line(&hints, theme)]
        }
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
    let names = &app.ws.names;
    let selected = app.selected();
    if let Some(selected) = selected.as_ref().filter(|s| s.writable()) {
        match selected.row.task.status {
            Status::Open => {
                add(Action::ToggleDone, "完了にする".to_string());
                for (action, key) in [
                    (Action::MoveToday, SectionKey::Today),
                    (Action::MoveNext, SectionKey::Next),
                    (Action::MoveWaiting, SectionKey::Waiting),
                    (Action::MoveSomeday, SectionKey::Someday),
                ] {
                    if selected.section == Some(key) {
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
        add(Action::Edit, "編集".to_string());
    }
    add(Action::Add, "追加".to_string());
    match app.screen {
        Screen::List => {
            if app.tab().writable() {
                add(Action::Sort, "並べ替え".to_string());
            }
            add(Action::OpenQueue, "Queue".to_string());
        }
        Screen::Queue => {
            if selected.is_some() {
                add(Action::Open, "一覧で開く".to_string());
            }
            let mode = if app.queue.wait { "期日別" } else { "待ち先別" };
            add(Action::QueueMode, mode.to_string());
            add(Action::Sort, "並べ替え".to_string());
            hints.push((Action::Back, "Esc".to_string(), "一覧へ".to_string()));
        }
    }
    let mut add = |action: Action, desc: String| {
        if let Some(key) = app.keymap.label(action) {
            hints.push((action, key, desc));
        }
    };
    add(Action::Help, "ヘルプ".to_string());
    add(Action::Quit, "終了".to_string());
    hints
}

/// 幅に収まるまで、重要でない案内から外す。`?` (ヘルプ) は外さない (外したキーもヘルプで分かるように)。
fn fit_hints(mut hints: Vec<(Action, String, String)>, width: usize) -> Vec<(String, String)> {
    const DROP_ORDER: [Action; 14] = [
        Action::Quit,
        Action::Sort,
        Action::OpenQueue,
        Action::Cancel,
        Action::Edit,
        Action::Back,
        Action::MoveSomeday,
        Action::MoveWaiting,
        Action::MoveNext,
        Action::MoveToday,
        Action::Add,
        Action::Open,
        Action::QueueMode,
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
        Action::Add => "タスクを追加する (追加先の初期値は開いているタブ)".to_string(),
        Action::Edit => "タスクを編集する".to_string(),
        Action::OpenQueue => "Queue を開く (期日・待ち先ごとの一覧)".to_string(),
        Action::QueueMode => "Queue: 期日別と待ち先別を切り替える".to_string(),
        Action::Open => "Queue: 選んでいるタスクを一覧で開く".to_string(),
        Action::Back => "一覧へ戻る".to_string(),
        Action::Help => "このヘルプ".to_string(),
        Action::Quit => "終了 (Ctrl-c でも)".to_string(),
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
    // Esc は割り当てを変えられない (入力欄やヘルプでも同じ意味で使うため)。
    entries.push((
        "Esc".to_string(),
        "押しかけのキー・入力・確認を取り消す。Queue からは一覧へ戻る (終了はしない)".to_string(),
    ));
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
        Action::Add,
        Action::Edit,
        Action::Sort,
        Action::OpenQueue,
        Action::QueueMode,
        Action::Open,
        Action::Help,
        Action::Quit,
    ]);
    actions
}

fn draw_help(frame: &mut Frame, app: &mut App, theme: Theme, area: Rect) {
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
    let visible = usize::from(height.saturating_sub(2));
    app.help_scroll = app.help_scroll.min(lines.len().saturating_sub(visible));
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((app.help_scroll as u16, 0))
            .block(Block::bordered().title(" ヘルプ ").border_style(theme.accent())),
        popup,
    );
}

/// フォームの項目名の幅 (「プロジェクト」「コンテキスト」)。
const FORM_LABEL_WIDTH: usize = 12;

fn field_label(field: Field) -> &'static str {
    match field {
        Field::Text => "本文",
        Field::Due => "期日",
        Field::Project => "プロジェクト",
        Field::Context => "コンテキスト",
        Field::Priority => "優先度",
        Field::Destination => "追加先",
        Field::Wait => "待ち先",
    }
}

fn destination_label(destination: Destination, app: &App) -> &str {
    match destination {
        Destination::Inbox => "Inbox",
        Destination::Section(key) => app.ws.names.name(key),
    }
}

/// 入力欄に見せる部分と、その中のカーソルの位置。入りきらなければ、カーソルが見えるよう先頭を隠す。
fn visible_input(text: &str, cursor_width: usize, width: usize) -> (String, usize) {
    let width = width.max(1);
    let mut skipped = 0;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        if cursor_width - skipped < width {
            break;
        }
        skipped += c.width().unwrap_or(0);
        start = i + c.len_utf8();
    }
    (truncate_plain(&text[start..], width), cursor_width - skipped)
}

/// 表示幅 `width` ごとに区切る。
fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut parts = vec![String::new()];
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width {
            parts.push(String::new());
            used = 0;
        }
        parts.last_mut().expect("never empty").push(c);
        used += w;
    }
    parts
}

/// `width` を超えた分を切り捨てる (`…` は付けない。入力欄の末尾にカーソルを置けるように)。
fn truncate_plain(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out
}

fn draw_form(frame: &mut Frame, form: &Form, app: &App, theme: Theme, area: Rect) {
    let width = area.width.saturating_sub(4).clamp(area.width.min(40), 100);
    let inner_width = usize::from(width.saturating_sub(2));
    // ガター (2)・項目名・空白 (2)
    let value_x = 2 + FORM_LABEL_WIDTH + 2;
    let value_width = inner_width.saturating_sub(value_x);

    let mut lines = vec![Line::raw("")];
    let mut cursor = None;
    for field in form.fields() {
        let focused = form.focus == field;
        let label = field_label(field);
        let mut spans = vec![
            if focused {
                Span::styled(GUTTER, theme.accent())
            } else {
                Span::raw(" ")
            },
            Span::raw(" "),
            Span::styled(
                format!(
                    "{label}{}  ",
                    " ".repeat(FORM_LABEL_WIDTH.saturating_sub(label.width()))
                ),
                if focused { theme.key() } else { theme.dim() },
            ),
        ];
        if field == Field::Destination {
            // 選んでいるものは前後の空白を括弧にする (色が無くても分かり、幅も変わらない)。
            for d in DESTINATIONS {
                let name = destination_label(d, app);
                if d == form.destination() {
                    spans.push(Span::styled(
                        format!("[{name}]"),
                        theme.key().add_modifier(Modifier::UNDERLINED),
                    ));
                } else {
                    spans.push(Span::raw(format!(" {name} ")));
                }
            }
        } else if let Some(input) = form.input(field) {
            let prefix = match field {
                Field::Project => "+",
                Field::Context => "@",
                _ => "",
            };
            spans.push(Span::styled(prefix, theme.dim()));
            let room = value_width.saturating_sub(prefix.width());
            let (shown, at) = visible_input(input.text(), input.cursor_width(), room);
            let shown_width = shown.width();
            spans.push(Span::raw(shown));
            if focused {
                cursor = Some((value_x + prefix.width() + at, lines.len()));
            }
            if field == Field::Due
                && let Some(due) = form.due()
            {
                let hint = match due {
                    Ok(date) => Span::styled(format!("  → {}", due_label(&date, app.today()).0), theme.dim()),
                    Err(_) => Span::styled(
                        "  読めない日付".to_string(),
                        theme.fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                };
                if shown_width + hint.width() <= room {
                    spans.push(hint);
                }
            }
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::raw(""));
    lines.push(Line::styled("  書き込む行", theme.dim()));
    match form.preview() {
        // 確定前に全体を確かめられるよう、切らずに折り返す。
        Ok(task) => {
            for part in wrap_plain(&preview_line(&task), inner_width.saturating_sub(4)) {
                lines.push(Line::raw(format!("  {part}")));
            }
        }
        Err(e) => lines.push(Line::styled(
            truncate(&format!("  error: {e}"), inner_width),
            theme.fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
    }
    let (candidates, chosen) = form.candidates();
    if !candidates.is_empty() {
        let prefix = if form.focus == Field::Project { "+" } else { "@" };
        let mut spans = vec![Span::styled("  候補 ", theme.dim())];
        let mut used = 7;
        for (i, c) in candidates.iter().enumerate() {
            let text = format!(" {prefix}{c} ");
            if used + text.width() > inner_width {
                spans.push(Span::styled("…", theme.dim()));
                break;
            }
            used += text.width();
            spans.push(if chosen == Some(i) {
                Span::styled(text, theme.key().add_modifier(Modifier::REVERSED))
            } else {
                Span::raw(text)
            });
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(spans));
    }

    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let title = if form.is_add() { " 追加 " } else { " 編集 " };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(title).border_style(theme.accent())),
        popup,
    );
    if let Some((x, y)) = cursor
        && y + 1 < usize::from(height)
    {
        let x = (popup.x + 1 + x as u16).min(popup.right().saturating_sub(2));
        frame.set_cursor_position((x, popup.y + 1 + y as u16));
    }
}

fn draw_queue(frame: &mut Frame, app: &mut App, theme: Theme, area: Rect) {
    let block = Block::new().borders(Borders::TOP).border_style(theme.dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let height = usize::from(inner.height);
    app.page_height = height;

    let q = &mut app.queue;
    let cursor_line = q.items().nth(q.cursor).map(|(i, _)| i);
    if let Some(line) = cursor_line {
        // 上へ移ったときは、そのタスクの見出しまで見せる。
        let mut top = line;
        while top > 0 && !matches!(q.lines[top - 1], queue::Line::Item(_)) {
            top -= 1;
        }
        if top < q.offset {
            q.offset = top;
        } else if height > 0 && line >= q.offset + height {
            q.offset = line + 1 - height;
        }
    }
    q.offset = q.offset.min(q.lines.len().saturating_sub(height.max(1)));

    let selected_line = cursor_line;
    let width = usize::from(inner.width);
    let lines: Vec<Line> = q
        .lines
        .iter()
        .enumerate()
        .skip(q.offset)
        .take(height)
        .map(|(i, line)| match line {
            queue::Line::Blank => Line::raw(""),
            queue::Line::Heading(text, kind) => {
                let style = match kind {
                    Heading::Overdue => theme.fg(Color::Red).add_modifier(Modifier::BOLD),
                    Heading::Today | Heading::Invalid | Heading::Person => {
                        theme.fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    }
                    Heading::Upcoming => theme.fg(Color::Blue).add_modifier(Modifier::BOLD),
                    Heading::Far => theme.dim(),
                };
                Line::styled(format!(" {text}"), style)
            }
            queue::Line::Empty(text) => Line::styled(format!("  {text}"), theme.fg(Color::Green)),
            queue::Line::Item(item) => queue_item_line(item, Some(i) == selected_line, width, theme),
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Queue のタスクの行。右端にどこにあるかを出す (inbox でまだ振り分けていないものが分かるように)。
fn queue_item_line(item: &queue::Item, selected: bool, width: usize, theme: Theme) -> Line<'static> {
    let task = &item.row.task;
    let base = if item.overdue {
        theme.fg(Color::Red)
    } else if item.far {
        theme.dim()
    } else {
        Style::new()
    };
    let head = vec![
        if selected {
            Span::styled(GUTTER, theme.accent())
        } else {
            Span::raw(" ")
        },
        Span::raw(" "),
        Span::styled(if item.overdue { "⚠ " } else { "▶ " }, base),
    ];
    let mut tail = Vec::new();
    for tag in [project_tag(task), context_tag(task)].into_iter().flatten() {
        tail.push(Span::styled(format!(" {tag}"), base));
    }
    if !item.suffix.is_empty() {
        tail.push(Span::styled(item.suffix.clone(), base));
    }
    let place = Span::styled(item.place.clone(), theme.dim());

    // 本文と右端の場所の間は 2 文字以上空ける。
    let fixed: usize = head.iter().chain(&tail).map(Span::width).sum::<usize>() + place.width() + 2;
    let content = truncate(&task.content, width.saturating_sub(fixed).max(MIN_CONTENT_WIDTH));
    let content_style = if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    };
    let used = fixed - 2 + content.width();
    let mut spans = head;
    spans.push(Span::styled(content, content_style));
    spans.extend(tail);
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used).max(2))));
    spans.push(place);
    Line::from(spans)
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
    fn a_long_input_scrolls_to_keep_the_cursor_visible() {
        assert_eq!(visible_input("abc", 3, 10), ("abc".to_string(), 3));
        assert_eq!(visible_input("abcdefgh", 8, 5), ("efgh".to_string(), 4));
        assert_eq!(visible_input("日本語です", 10, 6), ("です".to_string(), 4));
        // カーソルが先頭寄りなら、末尾を切るだけ。
        assert_eq!(visible_input("abcdefgh", 2, 5), ("abcde".to_string(), 2));
    }

    #[test]
    fn wrap_splits_by_display_width() {
        assert_eq!(wrap_plain("abcdef", 4), ["abcd", "ef"]);
        assert_eq!(wrap_plain("日本語", 4), ["日本", "語"]);
        assert_eq!(wrap_plain("", 4), [""]);
    }

    #[test]
    fn truncate_counts_display_width() {
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "ab…");
        assert_eq!(truncate("日本語", 5), "日本…");
        assert_eq!(truncate("日本語", 4), "日…");
    }
}
