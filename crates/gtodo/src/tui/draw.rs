//! 画面の描画。
//!
//! 色は飾りに留め、意味は記号・太字・下線でも分かるようにする (`NO_COLOR` や色の少ない端末のため)。

use chrono::NaiveDate;
use gtodo_core::ops::Destination;
use gtodo_core::search::Hit;
use gtodo_core::sections::SectionKey;
use gtodo_core::store::FileKind;
use gtodo_core::task::{Status, Task};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, Level, Mode, Row, Screen, TABS, Tab};
use super::form::{DESTINATIONS, Field, Form, preview_line};
use super::kanban;
use super::keymap::{Action, Key, sequence_label};
use super::markdown::{self, Inline, Urls};
use super::queue::{self, Heading};
use super::search::{self, StatusFilter};
use crate::render::{context_tag, project_tag};

/// 選んでいる行の左端の印。
pub const GUTTER: &str = "▌";
/// メッセージ欄の最大の行数。
const MAX_MESSAGE_LINES: usize = 6;
/// 本文を切り詰めるときも残す幅。これより狭いときはタグの方を切る。
const MIN_CONTENT_WIDTH: usize = 10;
/// Queue で本文を切り詰めるときも残す幅 (右端の場所を優先する)。
const MIN_QUEUE_CONTENT_WIDTH: usize = 4;

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
    let detail = detail_lines(app, theme, area);
    // 詳細欄は上に区切りの線を引く。
    let detail_height = if detail.is_empty() { 0 } else { detail.len() as u16 + 1 };
    let [tabs_area, list_area, detail_area, message_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(detail_height),
        Constraint::Length(messages.len() as u16),
        Constraint::Length(footer.len() as u16),
    ])
    .areas(area);
    if !detail.is_empty() {
        frame.render_widget(
            Paragraph::new(detail).block(Block::new().borders(Borders::TOP).border_style(theme.dim())),
            detail_area,
        );
    }

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
        Screen::Kanban => draw_kanban(frame, app, theme, tabs_area, list_area),
        Screen::Search => {
            let cursor = draw_search_bar(frame, app, theme, tabs_area);
            if app.search.typing && app.mode == Mode::Normal {
                frame.set_cursor_position(cursor);
            }
            draw_search(frame, app, theme, list_area);
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
    if let Mode::ChooseLink { links, cursor } = &app.mode {
        draw_link_choice(frame, links, *cursor, theme, area);
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

    let content_style = if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    };
    let content = Content {
        text: &task.content,
        ranges: &[],
        style: content_style,
        mark: content_style,
        min_width: MIN_CONTENT_WIDTH,
        theme,
    };
    layout_row(head, content, tail, None, width)
}

/// 行の本文 (幅に合わせて切り詰める部分)。
struct Content<'a> {
    text: &'a str,
    /// 強調する範囲 (検索で当たった部分)。
    ranges: &'a [std::ops::Range<usize>],
    style: Style,
    mark: Style,
    /// 狭くても残す幅。
    min_width: usize,
    theme: Theme,
}

/// 一覧・Queue・検索の行を組み立てる。本文が入りきらなければ `…` で切り、タグなどは残す。
/// `place` があれば右端にそろえて出す (本文との間は 2 文字以上空ける)。
fn layout_row(
    head: Vec<Span<'static>>,
    content: Content,
    tail: Vec<Span<'static>>,
    place: Option<Span<'static>>,
    width: usize,
) -> Line<'static> {
    let place_room = place.as_ref().map_or(0, |p| p.width() + 2);
    let fixed: usize = head.iter().chain(&tail).map(Span::width).sum::<usize>() + place_room;
    let room = width.saturating_sub(fixed).max(content.min_width);
    let pieces = content_pieces(
        content.text,
        content.ranges,
        content.style,
        content.mark,
        content.theme,
        Urls::Short,
    );
    let (body, body_width) = fit(pieces, room, content.style);
    let mut spans = head;
    spans.extend(body);
    spans.extend(tail);
    if let Some(place) = place {
        let used = fixed - place_room + body_width + place.width();
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used).max(2))));
        spans.push(place);
    }
    Line::from(spans)
}

/// 本文の後ろに出すタグと日付。
fn tail_spans(task: &Task, tab: Tab, today: NaiveDate, base: Style, theme: Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let value = |v: &Option<String>| v.as_deref().filter(|v| !v.is_empty()).map(str::to_string);
    let tags = Tags::of(task, today, base, theme);
    for tag in [tags.project, tags.context].into_iter().flatten() {
        spans.push(Span::raw(" "));
        spans.push(tag);
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
            for tag in [tags.due, tags.wait].into_iter().flatten() {
                spans.push(Span::raw("  "));
                spans.push(tag);
            }
        }
    }
    spans
}

/// タスクのタグの表示 (一覧の行とカンバンのカードで共有する。並べる順は呼び出し元が決める)。
struct Tags {
    project: Option<Span<'static>>,
    context: Option<Span<'static>>,
    /// 期日と残りの日数 (期限切れ・今日は強調する。完了したタスクでは強調しない)。
    due: Option<Span<'static>>,
    wait: Option<Span<'static>>,
}

impl Tags {
    fn of(task: &Task, today: NaiveDate, base: Style, theme: Theme) -> Tags {
        fn value(v: &Option<String>) -> Option<&str> {
            v.as_deref().filter(|v| !v.is_empty())
        }
        let due = value(&task.due).map(|due| {
            let (text, urgency) = due_label(due, today);
            let style = match (task.status, urgency) {
                (Status::Done, _) => base,
                (_, Urgency::Overdue) => theme.fg(Color::Red).add_modifier(Modifier::BOLD),
                (_, Urgency::Today) => theme.fg(Color::Yellow).add_modifier(Modifier::BOLD),
                (_, Urgency::Later) => base,
            };
            Span::styled(text, style)
        });
        Tags {
            project: project_tag(task).map(|p| Span::styled(p, base.patch(theme.fg(Color::Magenta)))),
            context: context_tag(task).map(|c| Span::styled(c, base.patch(theme.fg(Color::Green)))),
            due,
            wait: value(&task.wait).map(|w| Span::styled(format!("wait:{w}"), base.patch(theme.fg(Color::Blue)))),
        }
    }
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
    let (spans, _) = fit(vec![(text.to_string(), Style::new())], width, Style::new());
    spans.iter().map(|s| s.content.as_ref()).collect()
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
        Mode::ChooseLink { .. } => {
            let hints = [("1〜9/Enter", "開く"), ("↑/↓", "選ぶ"), ("Esc", "取り消す")]
                .map(|(k, d)| (k.to_string(), d.to_string()));
            vec![hint_line(&hints, theme)]
        }
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
        Mode::Normal if app.screen == Screen::Search && app.search.typing => {
            let hints = [("Enter", "結果へ"), ("↑/↓", "選ぶ"), ("Tab", "状態"), ("Esc", "一覧へ")]
                .map(|(k, d)| (k.to_string(), d.to_string()));
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
    // リンクは見るだけのタスク (done.md など) でも開ける。
    if selected
        .as_ref()
        .is_some_and(|s| !markdown::links(&s.row.task.content).is_empty())
    {
        add(Action::OpenLink, "リンクを開く".to_string());
    }
    add(Action::Add, "追加".to_string());
    match app.screen {
        Screen::List => {
            if app.tab().writable() {
                add(Action::Sort, "並べ替え".to_string());
            }
            add(Action::OpenQueue, "Queue".to_string());
            add(Action::OpenKanban, "カンバン".to_string());
        }
        Screen::Queue => {
            if selected.is_some() {
                add(Action::Open, "一覧で開く".to_string());
            }
            let mode = if app.queue.wait { "期日別" } else { "待ち先別" };
            add(Action::SwitchView, mode.to_string());
            add(Action::Sort, "並べ替え".to_string());
            hints.push((Action::Back, "Esc".to_string(), "一覧へ".to_string()));
        }
        Screen::Kanban => {
            if selected.is_some() {
                add(Action::Open, "一覧で開く".to_string());
            }
            let short = |action| {
                app.keymap
                    .keys_for(action)
                    .map(sequence_label)
                    .min_by_key(|l| l.width())
            };
            add(Action::Sort, "並べ替え".to_string());
            if let (Some(prev), Some(next)) = (short(Action::PrevTab), short(Action::NextTab)) {
                hints.push((Action::NextTab, format!("{prev}/{next}"), "列".to_string()));
            }
            hints.push((Action::Back, "Esc".to_string(), "一覧へ".to_string()));
        }
        Screen::Search => {
            if selected.is_some() {
                add(Action::Open, "一覧で開く".to_string());
            }
            add(Action::Search, "検索語".to_string());
            add(Action::SwitchView, "状態".to_string());
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
    const DROP_ORDER: [Action; 18] = [
        Action::Quit,
        Action::Sort,
        Action::OpenKanban,
        Action::OpenQueue,
        Action::Search,
        Action::OpenLink,
        Action::Cancel,
        Action::Edit,
        Action::Back,
        Action::MoveSomeday,
        Action::MoveWaiting,
        Action::MoveNext,
        Action::MoveToday,
        Action::Add,
        Action::Open,
        Action::SwitchView,
        Action::NextTab,
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
        Action::OpenKanban => "カンバンを開く (h/l などのタブのキーで列を移る)".to_string(),
        Action::OpenLink => "リンクを開く (複数あれば選ぶ)".to_string(),
        Action::Search => "検索 (本文の語・+project・@context・wait:。結果の中では入力欄へ戻る)".to_string(),
        Action::SwitchView => "表示を切り替える (Queue は期日別と待ち先別、検索はすべて・未完了・完了)".to_string(),
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
    Action::configurable()
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

    let content_style = if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    };
    // 狭くても右端の場所を残すため、一覧より本文を短くしてよい。
    let content = Content {
        text: &task.content,
        ranges: &[],
        style: content_style,
        mark: content_style,
        min_width: MIN_QUEUE_CONTENT_WIDTH,
        theme,
    };
    layout_row(head, content, tail, Some(place), width)
}

/// 検索の入力欄と状態の行。入力欄のカーソルの位置を返す。
fn draw_search_bar(frame: &mut Frame, app: &App, theme: Theme, area: Rect) -> (u16, u16) {
    let search = &app.search;
    let mut right = vec![];
    for status in [StatusFilter::All, StatusFilter::Open, StatusFilter::Done] {
        right.push(if status == search.status {
            Span::styled(format!("[{}]", status.label()), theme.key())
        } else {
            Span::styled(format!(" {} ", status.label()), theme.dim())
        });
    }
    right.push(Span::raw(format!(" {} 件 ", search.results.len())));
    let right_width: usize = right.iter().map(Span::width).sum();
    let room = usize::from(area.width).saturating_sub(right_width + 3);
    let (shown, at) = visible_input(search.input.text(), search.input.cursor_width(), room);
    let pad = room.saturating_sub(shown.width());
    let mut spans = vec![Span::styled(" /", theme.key()), Span::raw(" "), Span::raw(shown)];
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(right);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    (area.x + 3 + at as u16, area.y)
}

fn draw_search(frame: &mut Frame, app: &mut App, theme: Theme, area: Rect) {
    let block = Block::new().borders(Borders::TOP).border_style(theme.dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let height = usize::from(inner.height);
    app.page_height = height;
    let today = app.today();
    let search = &mut app.search;
    if search.results.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  一致するタスクはありません", theme.dim())),
            inner,
        );
        return;
    }
    if search.cursor < search.offset {
        search.offset = search.cursor;
    } else if height > 0 && search.cursor >= search.offset + height {
        search.offset = search.cursor + 1 - height;
    }
    search.offset = search.offset.min(search.results.len().saturating_sub(height.max(1)));
    let width = usize::from(inner.width);
    let lines: Vec<Line> = search
        .results
        .iter()
        .enumerate()
        .skip(search.offset)
        .take(height)
        .map(|(i, found)| {
            let entry = search.entry(found);
            // 当たった部分は、画面に出す行についてだけ求める。
            let ranges = search.query.highlight(&entry.row.task.content);
            search_row_line(entry, &found.hit, &ranges, i == search.cursor, today, width, theme)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// 検索の結果の行。当たった部分は太字と下線で示し (色に頼らない)、右端にどこにあるかを出す。
fn search_row_line(
    entry: &search::Entry,
    hit: &Hit,
    ranges: &[std::ops::Range<usize>],
    selected: bool,
    today: NaiveDate,
    width: usize,
    theme: Theme,
) -> Line<'static> {
    let task = &entry.row.task;
    let done = task.status == Status::Done;
    let base = if done { theme.dim() } else { Style::new() };
    let mark = theme
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
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
    let mut tail = Vec::new();
    for (tag, matched) in [(project_tag(task), hit.project), (context_tag(task), hit.context)] {
        if let Some(tag) = tag {
            tail.push(Span::raw(" "));
            tail.push(Span::styled(tag, if matched { mark } else { base }));
        }
    }
    if entry.file == FileKind::Done {
        if let Some(d) = task.completed_at.as_deref().filter(|d| !d.is_empty()) {
            tail.push(Span::styled(format!("  {d}"), theme.dim()));
        }
    } else if let Some(due) = task.due.as_deref().filter(|d| !d.is_empty()) {
        tail.push(Span::styled(format!("  {}", due_label(due, today).0), base));
    }
    if let Some(w) = task.wait.as_deref().filter(|w| !w.is_empty()) {
        tail.push(Span::raw("  "));
        tail.push(Span::styled(format!("wait:{w}"), if hit.wait { mark } else { base }));
    }
    let place = Span::styled(entry.place.clone(), theme.dim());

    let content_style = if selected {
        base.add_modifier(Modifier::BOLD)
    } else {
        base
    };
    let content = Content {
        text: &task.content,
        ranges,
        style: content_style,
        mark,
        min_width: MIN_QUEUE_CONTENT_WIDTH,
        theme,
    };
    layout_row(head, content, tail, Some(place), width)
}

/// 本文を markdown として整えた部分と、その装飾。`ranges` (検索で当たった元の本文の範囲) は
/// `mark` で強調する。URL を短くした部分などは、当たっていれば全体を強調する。
fn content_pieces(
    text: &str,
    ranges: &[std::ops::Range<usize>],
    base: Style,
    mark: Style,
    theme: Theme,
    urls: Urls,
) -> Vec<(String, Style)> {
    let mut pieces: Vec<(String, Style)> = Vec::new();
    let overlaps = |r: &std::ops::Range<usize>| ranges.iter().any(|m| m.start < r.end && r.start < m.end);
    for segment in markdown::render(text, urls, !theme.color).segments {
        let style = inline_style(base, segment.style, theme);
        if !segment.verbatim {
            let style = if overlaps(&segment.source) {
                style.patch(mark)
            } else {
                style
            };
            pieces.push((segment.text, style));
            continue;
        }
        // 元の本文そのままの部分は、当たった文字だけを強調する。
        for (i, c) in segment.text.char_indices() {
            let at = segment.source.start + i;
            let s = if ranges.iter().any(|r| r.contains(&at)) {
                style.patch(mark)
            } else {
                style
            };
            match pieces.last_mut() {
                Some((text, last)) if *last == s => text.push(c),
                _ => pieces.push((c.to_string(), s)),
            }
        }
    }
    pieces
}

/// markdown の装飾を文字の装飾にする。コードは色で示す (色を使わないときは `` ` `` を残してある)。
fn inline_style(base: Style, inline: Inline, theme: Theme) -> Style {
    let mut style = base;
    if inline.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if inline.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if inline.strike {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    if inline.code {
        style = style.patch(theme.fg(Color::LightYellow));
    }
    if inline.link {
        style = style.add_modifier(Modifier::UNDERLINED).patch(theme.fg(Color::Blue));
    }
    style
}

/// 幅 `width` に収まるよう切り詰める (超えた分は `…`)。作った幅も返す。
fn fit(pieces: Vec<(String, Style)>, width: usize, base: Style) -> (Vec<Span<'static>>, usize) {
    let total: usize = pieces.iter().map(|(t, _)| t.width()).sum();
    let cut = total > width;
    let limit = if cut { width.saturating_sub(1) } else { width };
    let mut spans = Vec::new();
    let mut used = 0;
    'pieces: for (text, style) in pieces {
        let mut kept = String::new();
        for c in text.chars() {
            let w = c.width().unwrap_or(0);
            if used + w > limit {
                if !kept.is_empty() {
                    spans.push(Span::styled(kept, style));
                }
                break 'pieces;
            }
            kept.push(c);
            used += w;
        }
        spans.push(Span::styled(kept, style));
    }
    if cut {
        spans.push(Span::styled("…", base));
        used += 1;
    }
    (spans, used)
}

/// 幅 `width` ごとに折り返し、`max_lines` 行を超えるなら最後の行の末尾を `…` にする。
fn wrap_pieces(pieces: Vec<(String, Style)>, width: usize, max_lines: usize, base: Style) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines: Vec<Vec<(String, Style)>> = vec![Vec::new()];
    let mut used = 0;
    for (text, style) in pieces {
        for c in text.chars() {
            let w = c.width().unwrap_or(0);
            if used + w > width {
                if lines.len() == max_lines {
                    // 入りきらない。最後の行の末尾を `…` にする。
                    let last = lines.last_mut().expect("never empty");
                    while used + 1 > width {
                        let Some((t, _)) = last.last_mut() else { break };
                        match t.pop() {
                            Some(removed) => used -= removed.width().unwrap_or(0),
                            None => {
                                last.pop();
                            }
                        }
                    }
                    last.push(("…".to_string(), base));
                    return lines.into_iter().map(to_line).collect();
                }
                lines.push(Vec::new());
                used = 0;
            }
            let line = lines.last_mut().expect("never empty");
            match line.last_mut() {
                Some((t, s)) if *s == style => t.push(c),
                _ => line.push((c.to_string(), style)),
            }
            used += w;
        }
    }
    lines.into_iter().map(to_line).collect()
}

fn to_line(pieces: Vec<(String, Style)>) -> Line<'static> {
    Line::from(pieces.into_iter().map(|(t, s)| Span::styled(t, s)).collect::<Vec<_>>())
}

/// カンバンの見出し (列の名前と枚数)。
fn kanban_title(app: &App, column: usize) -> String {
    let name = match kanban::SECTION_COLUMNS.get(column) {
        Some(key) => app.ws.names.name(*key),
        None => "Done",
    };
    format!(" {name} ({}) ", app.kanban.columns[column].len())
}

fn draw_kanban(frame: &mut Frame, app: &mut App, theme: Theme, title_area: Rect, area: Rect) {
    let (visible, column_width) = kanban::layout(usize::from(area.width));
    let first = kanban::first_visible(app.kanban.focus, visible, app.kanban.first_column);
    app.kanban.first_column = first;

    // 1 行目: 画面の名前と、入らずに左右へ送った列。
    let mut spans = vec![Span::styled(" Kanban", theme.accent().add_modifier(Modifier::BOLD))];
    let hidden = |range: std::ops::Range<usize>| -> Vec<String> {
        range.map(|c| kanban_title(app, c).trim().to_string()).collect()
    };
    let left = hidden(0..first);
    let right = hidden(first + visible..kanban::COLUMNS);
    if !left.is_empty() {
        spans.push(Span::styled(format!("   ◀ {}", left.join(" · ")), theme.dim()));
    }
    if !right.is_empty() {
        spans.push(Span::styled(format!("   {} ▶", right.join(" · ")), theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), title_area);

    let block = Block::new().borders(Borders::TOP).border_style(theme.dim());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let today = app.today();
    let mut page = 1;
    for slot in 0..visible.min(kanban::COLUMNS - first) {
        let column = first + slot;
        let x = inner.x + (slot * (column_width + 1)) as u16;
        let rect = Rect {
            x,
            y: inner.y,
            width: (column_width as u16).min(inner.right().saturating_sub(x)),
            height: inner.height,
        };
        let shown = draw_kanban_column(frame, app, theme, column, rect, today);
        if column == app.kanban.focus {
            page = shown;
        }
    }
    app.page_height = page.max(1);
}

/// 列を描き、入りきったカードの枚数を返す (半ページ・1 ページの移動に使う)。
fn draw_kanban_column(
    frame: &mut Frame,
    app: &mut App,
    theme: Theme,
    column: usize,
    area: Rect,
    today: NaiveDate,
) -> usize {
    let focused = column == app.kanban.focus;
    let title_style = if focused {
        theme.key().add_modifier(Modifier::UNDERLINED)
    } else {
        theme.dim().add_modifier(Modifier::BOLD)
    };
    let title = truncate(&kanban_title(app, column), usize::from(area.width));
    frame.render_widget(
        Paragraph::new(Line::styled(title, title_style)).alignment(ratatui::layout::Alignment::Center),
        Rect { height: 1, ..area },
    );
    let body = Rect {
        y: area.y + 1,
        height: area.height.saturating_sub(1),
        ..area
    };
    let cards = &app.kanban.columns[column];
    if cards.is_empty() {
        frame.render_widget(Paragraph::new(Span::styled("  (no tasks)", theme.dim())), body);
        return 0;
    }
    let inner_width = usize::from(area.width.saturating_sub(2));
    // 中身は見えるあたりのカードについてだけ作る (Done の列は数万枚になりうる)。高さを
    // 測るのに作ったものは、描くときに使い回す。
    let mut built: std::collections::HashMap<usize, Vec<Line<'static>>> = std::collections::HashMap::new();
    let height = |built: &mut std::collections::HashMap<usize, Vec<Line<'static>>>, i: usize| {
        built
            .entry(i)
            .or_insert_with(|| card_lines(&cards[i], inner_width, today, theme))
            .len() as u16
            + 2
    };

    // 選んでいるカードが全部見えるよう、表示する最初のカードをずらす。選んでいるカードから
    // 上へ、入るだけ数えたところが、選んでいるカードが見える一番上。
    let cursor = app.kanban.cursors[column].min(cards.len() - 1);
    let mut top = cursor;
    let mut used = height(&mut built, cursor);
    while top > 0 {
        let h = height(&mut built, top - 1);
        if used + h > body.height {
            break;
        }
        used += h;
        top -= 1;
    }
    let offset = app.kanban.offsets[column].clamp(top, cursor);
    app.kanban.offsets[column] = offset;

    let mut y = body.y;
    let mut shown = 0;
    for (i, card) in cards.iter().enumerate().skip(offset) {
        let mut height = height(&mut built, i);
        let mut sides = Borders::ALL;
        if y + height > body.bottom() {
            // 列より高いカードは、下の枠を除いて入る分だけ描く (選んでいるカードが見えなくならないように)。
            if shown > 0 || y >= body.bottom() {
                break;
            }
            height = body.bottom() - y;
            sides = Borders::TOP | Borders::LEFT | Borders::RIGHT;
        }
        let selected = focused && i == cursor;
        let border = if selected {
            Block::new()
                .borders(sides)
                .border_type(ratatui::widgets::BorderType::Thick)
                .border_style(theme.accent())
        } else {
            Block::new().borders(sides).border_style(theme.dim())
        };
        let lines = built
            .remove(&i)
            .unwrap_or_else(|| card_lines(card, inner_width, today, theme));
        frame.render_widget(Paragraph::new(lines).block(border), Rect { y, height, ..body });
        y += height;
        shown += 1;
    }
    shown
}

/// カードの中身。本文 (2 行まで) と、期日・待ち先・タグ・完了日の行。
/// `[ ]` は出さない (どの列にあるかが完了したかを表すため。Neovim 版と同じ)。
fn card_lines(card: &kanban::Card, width: usize, today: NaiveDate, theme: Theme) -> Vec<Line<'static>> {
    let task = &card.row.task;
    let from_done_md = card.file == FileKind::Done;
    let base = if from_done_md { theme.dim() } else { Style::new() };
    // 本文は 2 行まで (超えれば `…`)。markdown を整えて折り返す。
    let mut pieces = Vec::new();
    if let Some(p) = task.priority {
        pieces.push((format!("({p}) "), base.add_modifier(Modifier::BOLD)));
    }
    pieces.extend(content_pieces(&task.content, &[], base, base, theme, Urls::Short));
    let mut lines = wrap_pieces(pieces, width, kanban::BODY_LINES, base);

    let value = |v: &Option<String>| v.as_deref().filter(|v| !v.is_empty()).map(str::to_string);
    // 期日・待ち先・タグの順 (ROADMAP の決定事項)。完了したカードの期日は出さない (完了日を出す)。
    let t = Tags::of(task, today, base, theme);
    let due = t.due.filter(|_| task.status == Status::Open);
    let mut tags: Vec<Span> = [due, t.wait, t.project, t.context].into_iter().flatten().collect();
    if let Some(d) = value(&task.done) {
        tags.push(Span::styled(format!("done:{d}"), theme.dim()));
    } else if let Some(d) = value(&task.completed_at) {
        tags.push(Span::styled(format!("completed:{d}"), theme.dim()));
    }
    if !tags.is_empty() {
        // 1 行に収める (入らない分は `…`)。
        let mut spans: Vec<Span> = Vec::new();
        let mut used = 0;
        for (i, tag) in tags.into_iter().enumerate() {
            let sep = usize::from(i > 0);
            let w = tag.width();
            if used + sep + w > width {
                let room = width.saturating_sub(used + sep);
                if room > 0 {
                    if sep == 1 {
                        spans.push(Span::raw(" "));
                    }
                    spans.push(Span::styled(truncate(&tag.content, room), tag.style));
                }
                break;
            }
            if sep == 1 {
                spans.push(Span::raw(" "));
            }
            used += sep + w;
            spans.push(tag);
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// 詳細欄の行数の上限 (本文の行)。この下にタグの行が 1 行付く。
const DETAIL_BODY_LINES: usize = 3;
/// 詳細欄を出す端末の高さの下限 (低い端末では一覧の行を優先する)。
const DETAIL_MIN_HEIGHT: u16 = 16;

/// 選んでいるタスクの全文 (URL も短くしない) と、すべてのタグの行。カードや一覧の行が
/// 切れても、ここで全部読めるようにする。選んでいなければ空。
fn detail_lines(app: &App, theme: Theme, area: Rect) -> Vec<Line<'static>> {
    if area.height < DETAIL_MIN_HEIGHT {
        return Vec::new();
    }
    let Some(selected) = app.selected() else {
        return Vec::new();
    };
    let task = &selected.row.task;
    let width = usize::from(area.width);
    let mut pieces = vec![(
        if task.status == Status::Done { "[x] " } else { "[ ] " }.to_string(),
        theme.dim(),
    )];
    if let Some(p) = task.priority {
        pieces.push((format!("({p}) "), Style::new().add_modifier(Modifier::BOLD)));
    }
    pieces.extend(content_pieces(
        &task.content,
        &[],
        Style::new(),
        Style::new(),
        theme,
        Urls::Full,
    ));
    let mut lines = wrap_pieces(pieces, width, DETAIL_BODY_LINES, Style::new());

    fn value(v: &Option<String>) -> Option<&str> {
        v.as_deref().filter(|v| !v.is_empty())
    }
    let mut tags: Vec<String> = [project_tag(task), context_tag(task)].into_iter().flatten().collect();
    if let Some(due) = value(&task.due) {
        tags.push(due_label(due, app.today()).0);
    }
    for (name, v) in [
        ("wait", &task.wait),
        ("created", &task.created),
        ("completed_at", &task.completed_at),
        ("done", &task.done),
        ("cancelled", &task.cancelled),
    ] {
        if let Some(v) = value(v) {
            tags.push(format!("{name}:{v}"));
        }
    }
    if !tags.is_empty() {
        lines.push(Line::styled(truncate(&tags.join("  "), width), theme.dim()));
    }
    lines
}

/// 開くリンクを選ぶ小窓。
fn draw_link_choice(frame: &mut Frame, links: &[String], cursor: usize, theme: Theme, area: Rect) {
    let width = (area.width.saturating_sub(4)).min(100);
    let inner = usize::from(width.saturating_sub(4));
    let lines: Vec<Line> = links
        .iter()
        .enumerate()
        .map(|(i, url)| {
            let selected = i == cursor;
            let number = if i < 9 { format!("{}", i + 1) } else { " ".to_string() };
            Line::from(vec![
                if selected {
                    Span::styled(GUTTER, theme.accent())
                } else {
                    Span::raw(" ")
                },
                Span::styled(format!("{number} "), theme.key()),
                Span::styled(
                    truncate(url, inner.saturating_sub(3)),
                    if selected {
                        Style::new().add_modifier(Modifier::BOLD)
                    } else {
                        Style::new()
                    },
                ),
            ])
        })
        .collect();
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" リンクを開く ").border_style(theme.accent())),
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

    /// `content_pieces` で整えて `fit` で切る (一覧の行と同じ)。
    fn highlighted(
        text: &str,
        ranges: &[std::ops::Range<usize>],
        width: usize,
        base: Style,
        mark: Style,
    ) -> (Vec<Span<'static>>, usize) {
        let pieces = content_pieces(text, ranges, base, mark, Theme { color: false }, Urls::Short);
        fit(pieces, width, base)
    }

    fn plain(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn wrapping_keeps_two_lines_and_marks_the_cut() {
        let p = |t: &str| vec![(t.to_string(), Style::new())];
        assert_eq!(plain(&wrap_pieces(p("abcdef"), 3, 2, Style::new())), ["abc", "def"]);
        assert_eq!(plain(&wrap_pieces(p("abcdefg"), 3, 2, Style::new())), ["abc", "de…"]);
        assert_eq!(
            plain(&wrap_pieces(
                p("(A) とても長いタスクの本文がここに続く"),
                12,
                2,
                Style::new()
            )),
            ["(A) とても長", "いタスクの…"]
        );
        assert_eq!(plain(&wrap_pieces(p("短い"), 12, 2, Style::new())), ["短い"]);
    }

    #[test]
    fn markdown_in_a_row_is_styled_and_a_match_inside_a_url_marks_the_link() {
        let (base, mark) = (Style::new(), Style::new().add_modifier(Modifier::REVERSED));
        let text = "**重要** [仕様](https://example.com/spec/v2) を読む";
        // 「spec」は URL の中にだけある。見えているリンクの文字を強調する。
        let at = text.find("spec").unwrap();
        let (spans, _) = highlighted(text, &[std::ops::Range { start: at, end: at + 4 }], 40, base, mark);
        let parts: Vec<(String, bool, bool)> = spans
            .iter()
            .map(|s| {
                (
                    s.content.to_string(),
                    s.style.add_modifier.contains(Modifier::BOLD),
                    s.style.add_modifier.contains(Modifier::REVERSED),
                )
            })
            .collect();
        assert_eq!(
            parts,
            [
                ("重要".to_string(), true, false),
                (" ".to_string(), false, false),
                ("仕様".to_string(), false, true),
                (" を読む".to_string(), false, false),
            ]
        );
    }

    #[test]
    fn highlights_are_cut_with_the_content_and_the_ellipsis_is_not_marked() {
        use std::ops::Range;

        let (base, mark) = (Style::new(), Style::new().add_modifier(Modifier::UNDERLINED));
        let text = "会議の資料を作る";
        // 「資料」は 6〜12 バイト目。幅 9 では「会議の資…」になる。
        let (spans, width) = highlighted(text, &[Range { start: 9, end: 15 }], 9, base, mark);
        let parts: Vec<(String, bool)> = spans.iter().map(|s| (s.content.to_string(), s.style == mark)).collect();
        assert_eq!(
            parts,
            [
                ("会議の".to_string(), false),
                ("資".to_string(), true),
                ("…".to_string(), false)
            ]
        );
        assert_eq!(width, 9);
        // 当たった部分が切った先だけにあれば、強調は出ない。
        let (spans, _) = highlighted(text, &[Range { start: 18, end: 24 }], 9, base, mark);
        assert!(spans.iter().all(|s| s.style == base));
        // 入りきれば切らない。
        let (spans, width) = highlighted("abc", &[Range { start: 1, end: 2 }], 10, base, mark);
        assert_eq!(spans.len(), 3);
        assert_eq!(width, 3);
    }

    #[test]
    fn truncate_counts_display_width() {
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "ab…");
        assert_eq!(truncate("日本語", 5), "日本…");
        assert_eq!(truncate("日本語", 4), "日…");
    }
}
