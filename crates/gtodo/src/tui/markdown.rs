//! タスクの本文の markdown を、表示するときだけ整える (ファイル・フォーム・CLI の出力は元のまま)。
//!
//! 本文は 1 行なので、文中の装飾 (太字・斜体・コード・取り消し線・リンク) だけを読む。
//! 先頭の `1. `・`# `・`- ` などを見出しやリストとして読まないよう、段落の途中として読む。
//! 画像と HTML は書かれたままの文字を出す。URL (`<URL>` と、そのまま書いたもの) は
//! 「ホスト名/…/末尾」に短くする。markdown で書いたリンクで本文が長くなっても、一覧の行や
//! カードに本文が残るようにするため。

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};

/// 文字の装飾。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Inline {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
}

/// 表示する部分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub style: Inline,
    /// 元の本文のどこを表しているか。検索で当たった場所を強調するのに使う。
    pub source: Range<usize>,
    /// `text` が元の本文の `source` そのものか。そうなら当たった文字だけを強調できる。
    /// そうでなければ (短くした URL・リンクの文字など)、当たれば全体を強調する。
    pub verbatim: bool,
}

/// 整えた本文と、その中のリンクの行き先 (現れた順、重複なし)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rendered {
    pub segments: Vec<Segment>,
    pub links: Vec<String>,
}

/// URL の見せ方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urls {
    /// 「ホスト名/…/末尾」に短くする (一覧の行・カード)。
    Short,
    /// そのまま出し、markdown のリンクは文字の後ろに行き先を添える (詳細欄。全部読めるように)。
    Full,
}

/// 段落の途中として読ませるために前に置く文字。
const PREFIX: &str = "x ";

/// `content` を整える。`code_marks` が真ならコードの前後の `` ` `` を残す (色を使わないとき)。
pub fn render(content: &str, urls: Urls, code_marks: bool) -> Rendered {
    let source = format!("{PREFIX}{content}");
    let shift = |r: Range<usize>| r.start.saturating_sub(PREFIX.len())..r.end.saturating_sub(PREFIX.len());
    let mut out = Rendered::default();
    let mut style = Inline::default();
    // 読んでいるリンク (行き先、元の範囲、自動リンクか)。
    let mut link: Option<(String, Range<usize>, bool)> = None;
    // 画像の中 (書かれたままの文字を出したので、中身は飛ばす)。
    let mut image_depth = 0;
    // `~` 1 つの組の中 (書かれたままの文字を出したので、中身は飛ばす)。
    let mut tilde_depth = 0;

    for (event, range) in Parser::new_ext(&source, Options::ENABLE_STRIKETHROUGH).into_offset_iter() {
        if tilde_depth > 0 {
            match event {
                Event::Start(Tag::Strikethrough) => tilde_depth += 1,
                Event::End(TagEnd::Strikethrough) => tilde_depth -= 1,
                _ => {}
            }
            continue;
        }
        if image_depth > 0 {
            match event {
                Event::Start(Tag::Image { .. }) => image_depth += 1,
                Event::End(TagEnd::Image) => image_depth -= 1,
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(Tag::Strong) => style.bold = true,
            Event::End(TagEnd::Strong) => style.bold = false,
            Event::Start(Tag::Emphasis) => style.italic = true,
            Event::End(TagEnd::Emphasis) => style.italic = false,
            // `~~` だけを取り消し線にする。pulldown-cmark は `~` 1 つの組も取り消し線にするが、
            // 「1~2時間、3~4日」のような範囲の書き方まで消えてしまうため、書かれたまま出す。
            Event::Start(Tag::Strikethrough) if !source[range.clone()].starts_with("~~") => {
                tilde_depth = 1;
                let range = shift(range);
                push(&mut out, content[range.clone()].to_string(), style, range, true);
            }
            Event::Start(Tag::Strikethrough) => style.strike = true,
            Event::End(TagEnd::Strikethrough) => style.strike = false,
            Event::Start(Tag::Link {
                link_type, dest_url, ..
            }) => {
                let auto = matches!(link_type, LinkType::Autolink | LinkType::Email);
                // `<a@b.c>` の行き先にはスキームが付かない。開けるように付ける。
                let dest = match link_type {
                    LinkType::Email => format!("mailto:{dest_url}"),
                    _ => dest_url.to_string(),
                };
                add_link(&mut out.links, &dest);
                link = Some((dest, shift(range), auto));
            }
            Event::End(TagEnd::Link) => {
                let Some((dest, range, auto)) = link.take() else {
                    continue;
                };
                let style = Inline { link: true, ..style };
                if auto {
                    let text = match urls {
                        Urls::Short => shorten_url(&dest),
                        Urls::Full => dest.clone(),
                    };
                    let text = text.strip_prefix("mailto:").map_or(text.clone(), str::to_string);
                    push(&mut out, text, style, range, false);
                } else if urls == Urls::Full {
                    push(&mut out, format!(" ({dest})"), Inline::default(), range, false);
                }
            }
            Event::Start(Tag::Image { .. }) => {
                image_depth = 1;
                let range = shift(range);
                push(&mut out, content[range.clone()].to_string(), style, range, true);
            }
            Event::Text(text) => {
                let mut text: &str = &text;
                let mut range = range;
                if range.start == 0 {
                    // 前に置いた文字を除く。
                    text = text.strip_prefix(PREFIX).unwrap_or(text);
                    range.start = PREFIX.len();
                }
                let range = shift(range);
                match &link {
                    // 自動リンクの文字は URL そのもの。終わりで短くして出す。
                    Some((_, _, true)) => {}
                    Some((_, link_range, false)) => {
                        let style = Inline { link: true, ..style };
                        push(&mut out, text.to_string(), style, link_range.clone(), false);
                    }
                    None => {
                        let verbatim = content.get(range.clone()) == Some(text);
                        push_text(&mut out, text, style, range, verbatim, urls);
                    }
                }
            }
            Event::Code(code) => {
                let range = shift(range);
                let text = if code_marks {
                    content[range.clone()].to_string()
                } else {
                    code.to_string()
                };
                let verbatim = code_marks;
                push(&mut out, text, Inline { code: true, ..style }, range, verbatim);
            }
            Event::InlineHtml(_) | Event::Html(_) | Event::FootnoteReference(_) | Event::InlineMath(_) => {
                let range = shift(range);
                push(&mut out, content[range.clone()].to_string(), style, range, true);
            }
            Event::SoftBreak | Event::HardBreak => {
                let range = shift(range);
                push(&mut out, " ".to_string(), style, range, false);
            }
            _ => {}
        }
    }
    out
}

/// リンクの行き先 (`gx` で開くもの)。現れた順、重複なし。
pub fn links(content: &str) -> Vec<String> {
    render(content, Urls::Full, false).links
}

fn add_link(links: &mut Vec<String>, url: &str) {
    if !links.iter().any(|l| l == url) {
        links.push(url.to_string());
    }
}

fn push(out: &mut Rendered, text: String, style: Inline, source: Range<usize>, verbatim: bool) {
    if !text.is_empty() {
        out.segments.push(Segment {
            text,
            style,
            source,
            verbatim,
        });
    }
}

/// 文字の中の、そのまま書いた URL を見つけてリンクにする。
fn push_text(out: &mut Rendered, text: &str, style: Inline, source: Range<usize>, verbatim: bool, urls: Urls) {
    let mut rest = 0;
    for (start, end) in bare_urls(text) {
        let at = |i: usize| if verbatim { source.start + i } else { source.start };
        let to = |i: usize| if verbatim { source.start + i } else { source.end };
        push(out, text[rest..start].to_string(), style, at(rest)..to(start), verbatim);
        let url = &text[start..end];
        add_link(&mut out.links, url);
        let shown = match urls {
            Urls::Short => shorten_url(url),
            Urls::Full => url.to_string(),
        };
        let link_style = Inline { link: true, ..style };
        push(
            out,
            shown,
            link_style,
            at(start)..to(end),
            urls == Urls::Full && verbatim,
        );
        rest = end;
    }
    let tail = if verbatim {
        source.start + rest..source.end
    } else {
        source
    };
    push(out, text[rest..].to_string(), style, tail, verbatim);
}

/// そのまま書いた URL (`http://`・`https://` で始まり、空白か ASCII でない文字の前まで) の範囲。
/// 日本語の文では URL の後に空白を置かないことが多いため、ASCII でない文字で区切る (日本語を含む
/// URL は、ふつう `%` で書かれている)。後ろの句読点と、対になっていない閉じ括弧は URL に含めない
/// (文の終わりや括弧書きのため)。
fn bare_urls(text: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(i) = ["https://", "http://"]
        .iter()
        .filter_map(|p| text[from..].find(p).map(|i| from + i))
        .min()
    {
        // 単語の途中 (`xhttp://`) は URL とみなさない。
        let starts_word = text[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let mut end = text[i..]
            .find(|c: char| c.is_whitespace() || !c.is_ascii())
            .map_or(text.len(), |e| i + e);
        while let Some(last) = text[i..end].chars().next_back() {
            let trim = match last {
                '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => true,
                ')' => text[i..end].matches('(').count() < text[i..end].matches(')').count(),
                _ => false,
            };
            if !trim {
                break;
            }
            end -= last.len_utf8();
        }
        let scheme = if text[i..].starts_with("https://") { 8 } else { 7 };
        if starts_word && end > i + scheme {
            found.push((i, end));
        }
        from = end.max(i + scheme);
    }
    found
}

/// URL を「ホスト名/…/末尾」に短くする。パスが 1 段までならそのまま (スキームは除く)。
pub fn shorten_url(url: &str) -> String {
    let Some((_, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let rest = rest.trim_end_matches('/');
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let Some(host) = parts.next() else {
        return url.to_string();
    };
    let path: Vec<&str> = parts.collect();
    match path.as_slice() {
        [] => host.to_string(),
        [one] => format!("{host}/{one}"),
        [.., last] => format!("{host}/…/{last}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(content: &str) -> Vec<(String, Inline)> {
        render(content, Urls::Short, false)
            .segments
            .into_iter()
            .map(|s| (s.text, s.style))
            .collect()
    }

    fn text(content: &str, urls: Urls) -> String {
        render(content, urls, false)
            .segments
            .iter()
            .map(|s| s.text.as_str())
            .collect()
    }

    const BOLD: Inline = Inline {
        bold: true,
        italic: false,
        strike: false,
        code: false,
        link: false,
    };

    #[test]
    fn inline_styles_become_decorations() {
        assert_eq!(
            shown("**大事** な件"),
            [("大事".into(), BOLD), (" な件".into(), Inline::default())]
        );
        assert_eq!(
            text("*斜体* と ~~消した~~ と `code`", Urls::Short),
            "斜体 と 消した と code"
        );
        let r = render("`code` だけ", Urls::Short, true);
        assert_eq!(r.segments[0].text, "`code`", "code marks are kept without colour");
        assert!(r.segments[0].style.code);
    }

    #[test]
    fn only_double_tildes_strike_through() {
        assert_eq!(
            text("作業 1~2時間、確認 3~4日", Urls::Short),
            "作業 1~2時間、確認 3~4日"
        );
        let r = render("~~取り消し~~ と ~範囲~", Urls::Short, false);
        assert!(r.segments[0].style.strike);
        assert_eq!(r.segments[0].text, "取り消し");
        assert_eq!(r.segments.last().unwrap().text, "~範囲~");
        assert!(!r.segments.last().unwrap().style.strike);
    }

    #[test]
    fn block_markers_at_the_start_are_kept_as_text() {
        for content in [
            "1. 手順",
            "# 見出しではない",
            "- リストではない",
            "> 引用ではない",
            "---",
        ] {
            assert_eq!(text(content, Urls::Short), content);
        }
    }

    #[test]
    fn links_show_their_text_and_urls_are_shortened() {
        let r = render(
            "[仕様書](https://example.com/docs/spec/v2) と https://example.com/a/b/c. 参照 <https://x.org/y>",
            Urls::Short,
            false,
        );
        let text: String = r.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "仕様書 と example.com/…/c. 参照 x.org/y");
        assert_eq!(
            r.links,
            [
                "https://example.com/docs/spec/v2",
                "https://example.com/a/b/c",
                "https://x.org/y"
            ]
        );
        assert!(r.segments[0].style.link);
    }

    #[test]
    fn the_detail_view_shows_full_urls() {
        assert_eq!(
            text(
                "[仕様書](https://example.com/docs/spec/v2) https://a.com/x/y",
                Urls::Full
            ),
            "仕様書 (https://example.com/docs/spec/v2) https://a.com/x/y"
        );
    }

    #[test]
    fn images_and_html_are_shown_as_written() {
        assert_eq!(
            text("図 ![図1](img/a.png) を見る", Urls::Short),
            "図 ![図1](img/a.png) を見る"
        );
        assert_eq!(text("<b>太字</b>", Urls::Short), "<b>太字</b>");
    }

    #[test]
    fn segments_point_back_to_the_source() {
        let content = "前 [リンク](https://e.com/a/b) 後";
        let r = render(content, Urls::Short, false);
        let link = r.segments.iter().find(|s| s.style.link).unwrap();
        assert_eq!(&content[link.source.clone()], "[リンク](https://e.com/a/b)");
        assert!(!link.verbatim);
        let last = r.segments.last().unwrap();
        assert_eq!(&content[last.source.clone()], " 後");
        assert!(last.verbatim);
        let bare = "見る https://e.com/a/b/c 今日";
        let r = render(bare, Urls::Short, false);
        let url = r.segments.iter().find(|s| s.style.link).unwrap();
        assert_eq!(&bare[url.source.clone()], "https://e.com/a/b/c");
    }

    #[test]
    fn urls_are_shortened_to_host_and_tail() {
        assert_eq!(shorten_url("https://example.com"), "example.com");
        assert_eq!(shorten_url("https://example.com/"), "example.com");
        assert_eq!(shorten_url("https://example.com/docs"), "example.com/docs");
        assert_eq!(shorten_url("https://example.com/a/b/c.html"), "example.com/…/c.html");
        assert_eq!(shorten_url("https://example.com/a/b/"), "example.com/…/b");
        assert_eq!(
            shorten_url("https://www.google.com/search?q=x"),
            "www.google.com/search?q=x"
        );
        assert_eq!(shorten_url("mailto:a@b.c"), "mailto:a@b.c");
    }

    #[test]
    fn bare_urls_stop_at_spaces_and_trailing_punctuation() {
        fn found(t: &str) -> Vec<&str> {
            bare_urls(t).into_iter().map(|(s, e)| &t[s..e]).collect()
        }
        assert_eq!(found("見て https://a.com/x。次"), ["https://a.com/x"]);
        assert_eq!(
            found("(https://a.com/wiki/Foo_(bar))"),
            ["https://a.com/wiki/Foo_(bar)"]
        );
        assert_eq!(found("(https://a.com/x)"), ["https://a.com/x"]);
        assert_eq!(found("「https://a.com/x」を見る"), ["https://a.com/x"]);
        assert_eq!(found("https://a.com/%E6%97%A5"), ["https://a.com/%E6%97%A5"]);
        assert_eq!(found("a http://b.c d https://e.f"), ["http://b.c", "https://e.f"]);
        assert_eq!(found("xhttps://a.com"), Vec::<&str>::new());
        assert_eq!(found("https:// だけ"), Vec::<&str>::new());
    }

    #[test]
    fn email_autolinks_open_as_mailto() {
        let r = render("連絡 <a@b.example>", Urls::Short, false);
        assert_eq!(r.links, ["mailto:a@b.example"]);
        assert_eq!(r.segments.last().unwrap().text, "a@b.example");
    }

    #[test]
    fn links_are_listed_once_in_order() {
        assert_eq!(
            links("https://a.com/1 [x](https://b.com) https://a.com/1"),
            ["https://a.com/1", "https://b.com"]
        );
        assert!(links("リンクなし").is_empty());
    }
}
