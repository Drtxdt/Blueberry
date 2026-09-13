//! Pure ANSI menu rendering.
//!
//! The renderer only returns complete lines. It never emits cursor movement,
//! clears, or terminal queries; the host owns placement and restoration of
//! the overlay.

use std::env;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::config::{Config, UiConfig, is_valid_color};
use crate::model::{Candidate, CandidateKind};

/// A fully rendered menu. `width` is the maximum display width of every line
/// after ANSI sequences are removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuFrame {
    pub lines: Vec<String>,
    pub width: u16,
}

#[derive(Clone, Copy, Default)]
pub struct MenuState<'a> {
    pub incomplete: bool,
    pub details: bool,
    pub detail_page: usize,
    pub argument_hint: Option<&'a str>,
    pub searching: bool,
    pub diagnostic: Option<&'a str>,
}

pub fn render_with_state(
    candidates: &[Candidate],
    selected: usize,
    query: &str,
    available_width: u16,
    config: &Config,
    state: MenuState,
    available_rows: usize,
) -> MenuFrame {
    let mut adjusted = Config {
        ui: config.ui.clone(),
        completion: config.completion.clone(),
        ..Default::default()
    };
    let footer_rows = usize::from(config.ui.status_bar)
        + usize::from(state.searching)
        + usize::from(state.argument_hint.is_some_and(|s| !s.is_empty()))
        + if state.details { 4 } else { 0 };
    let border_rows = if config.ui.border == "none" { 0 } else { 2 };
    if available_rows == 0 {
        return MenuFrame {
            lines: Vec::new(),
            width: 0,
        };
    }
    if available_rows < border_rows + 1 {
        adjusted.ui.border = "none".into();
    }
    adjusted.ui.max_rows = adjusted.ui.max_rows.min(
        available_rows
            .saturating_sub(footer_rows + border_rows)
            .max(1),
    );
    let mut frame = render(candidates, selected, query, available_width, &adjusted);
    frame.lines.truncate(available_rows);
    let candidate = candidates.get(selected);
    if frame.width == 0 {
        return frame;
    }
    let mut push = |text: &str| {
        if frame.lines.len() < available_rows {
            let text = truncate_to_width(&sanitize_text(text), frame.width as usize);
            let padded = format!(
                "{}{}",
                text,
                " ".repeat((frame.width as usize).saturating_sub(display_width(&text)))
            );
            frame.lines.push(paint_span(
                &padded,
                FragmentRole::Description,
                false,
                &config.ui,
            ));
        }
    };
    if let Some(hint) = state.argument_hint.filter(|s| !s.is_empty()) {
        push(hint);
    }
    if state.searching {
        push(&format!(
            "用途搜索 · Tab 插入 · Esc 退出{}",
            candidates
                .get(selected)
                .filter(|c| !c.match_reason.is_empty())
                .map(|c| format!(" · {}", c.match_reason))
                .unwrap_or_default()
        ));
    }
    let Some(candidate) = candidate else {
        return frame;
    };
    if config.ui.status_bar {
        let source = if candidate.source.is_empty() {
            "内置"
        } else {
            &candidate.source
        };
        if let Some(diagnostic) = state.diagnostic {
            push(diagnostic);
        } else {
            push(&format!(
                "{}/{}{} · F1 详情 · {}",
                selected + 1,
                candidates.len(),
                if state.incomplete {
                    " · 正在加载"
                } else {
                    ""
                },
                source,
            ));
        }
    }
    if state.details {
        push(&format!("{}：{}", candidate.label, candidate.description));
        for line in detail_lines(candidate, frame.width as usize)
            .iter()
            .skip(state.detail_page * 3)
            .take(3)
        {
            push(line);
        }
        if candidate.detail.is_empty() {
            push("暂无更多说明；接受候选只插入文本。");
        }
    }
    frame
}

pub fn detail_lines(candidate: &Candidate, width: usize) -> Vec<String> {
    let full = format!(
        "{}\n来源：{}\n说明来源：{}\n{}",
        candidate.detail, candidate.source, candidate.description_source, candidate.match_reason
    );
    let mut rows = Vec::new();
    for line in full.lines() {
        let line = sanitize_text(line);
        let mut row = String::new();
        let mut cells = 0;
        for grapheme in line.graphemes(true) {
            let size = display_width(grapheme);
            if cells + size > width.max(1) && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                cells = 0;
            }
            row.push_str(grapheme);
            cells += size;
        }
        rows.push(row);
    }
    rows
}

/// Render a candidate list as a bounded, terminal-safe frame.
///
/// `selected` is clamped to the visible candidate range. When more candidates
/// exist than fit in a page, the page follows the selection while keeping it
/// visible. Candidate text is sanitized before any styling is added, so input
/// cannot inject terminal escapes into the returned frame.
pub fn render(
    candidates: &[Candidate],
    selected: usize,
    query: &str,
    available_width: u16,
    config: &Config,
) -> MenuFrame {
    let candidate_count = candidates.len().min(config.completion.max_results);
    if candidate_count == 0 || available_width == 0 {
        return MenuFrame {
            lines: Vec::new(),
            width: 0,
        };
    }

    let desired_width = if config.ui.width == 0 {
        100
    } else {
        config.ui.width
    };
    let frame_width = (available_width as usize)
        .min(desired_width)
        .min(u16::MAX as usize);
    if frame_width == 0 {
        return MenuFrame {
            lines: Vec::new(),
            width: 0,
        };
    }

    let selected = selected.min(candidate_count - 1);
    let page_rows = config.ui.max_rows.min(candidate_count);
    if page_rows == 0 {
        return MenuFrame {
            lines: Vec::new(),
            width: 0,
        };
    }

    let page_start = selected
        .saturating_sub(page_rows / 2)
        .min(candidate_count - page_rows);
    let query = sanitize_text(query);

    // A one-column terminal cannot contain a useful border. Falling back to
    // a borderless row keeps the candidate text available at that width.
    let border = border_chars(&config.ui.border);
    let bordered = border.is_some() && frame_width >= 2;
    let content_width = if bordered {
        frame_width.saturating_sub(2)
    } else {
        frame_width
    };

    let mut lines = Vec::with_capacity(page_rows + usize::from(bordered) * 2);
    if let Some(border) = border.filter(|_| bordered) {
        let top = format!(
            "{}{}{}",
            border.top_left,
            border.horizontal.repeat(content_width),
            border.top_right
        );
        lines.push(paint_fragment(
            &top,
            FragmentRole::Border,
            false,
            &query,
            &config.ui,
        ));
    }

    let label_column = candidates[page_start..page_start + page_rows]
        .iter()
        .map(|c| display_width(&c.label))
        .max()
        .unwrap_or(0)
        .min(content_width.saturating_sub(8) / 2);
    for (index, candidate) in candidates[page_start..page_start + page_rows]
        .iter()
        .enumerate()
    {
        let is_selected = page_start + index == selected;
        let row = render_candidate(
            candidate,
            is_selected,
            &query,
            content_width,
            &config.ui,
            label_column,
        );
        if let Some(border) = border.filter(|_| bordered) {
            let mut line = String::new();
            line.push_str(&paint_fragment(
                border.vertical,
                FragmentRole::Border,
                false,
                &query,
                &config.ui,
            ));
            line.push_str(&row);
            line.push_str(&paint_fragment(
                border.vertical,
                FragmentRole::Border,
                false,
                &query,
                &config.ui,
            ));
            lines.push(line);
        } else {
            lines.push(row);
        }
    }

    if let Some(border) = border.filter(|_| bordered) {
        let bottom = format!(
            "{}{}{}",
            border.bottom_left,
            border.horizontal.repeat(content_width),
            border.bottom_right
        );
        lines.push(paint_fragment(
            &bottom,
            FragmentRole::Border,
            false,
            &query,
            &config.ui,
        ));
    }

    MenuFrame {
        lines,
        width: frame_width as u16,
    }
}

/// Render a small themed preview useful for configuration UIs and diagnostics.
pub fn preview(config: &Config) -> String {
    let candidates = [
        Candidate {
            label: "Get-ChildItem".to_owned(),
            insert_text: "Get-ChildItem".to_owned(),
            description: "列出目录中的项目".to_owned(),
            kind: CandidateKind::Cmdlet,
            ..Default::default()
        },
        Candidate {
            label: "Get-Content".to_owned(),
            insert_text: "Get-Content".to_owned(),
            description: "读取文件内容".to_owned(),
            kind: CandidateKind::Cmdlet,
            ..Default::default()
        },
    ];
    let mut previews = Vec::new();
    for style in ["unicode", "nerd"] {
        let mut config = config.clone();
        config.ui.icon_style = style.into();
        previews.push(format!(
            "{style}\n{}",
            render(&candidates, 0, "Get-", 60, &config).lines.join("\n")
        ));
    }
    previews.join("\n")
}

#[derive(Copy, Clone)]
struct BorderChars {
    top_left: &'static str,
    top_right: &'static str,
    bottom_left: &'static str,
    bottom_right: &'static str,
    horizontal: &'static str,
    vertical: &'static str,
}

fn border_chars(border: &str) -> Option<BorderChars> {
    match border {
        "rounded" => Some(BorderChars {
            top_left: "╭",
            top_right: "╮",
            bottom_left: "╰",
            bottom_right: "╯",
            horizontal: "─",
            vertical: "│",
        }),
        "square" => Some(BorderChars {
            top_left: "┌",
            top_right: "┐",
            bottom_left: "└",
            bottom_right: "┘",
            horizontal: "─",
            vertical: "│",
        }),
        _ => None,
    }
}

#[derive(Copy, Clone)]
enum FragmentRole {
    Base,
    Icon(CandidateKind),
    Description,
    Match,
    Border,
}

fn render_candidate(
    candidate: &Candidate,
    selected: bool,
    query: &str,
    width: usize,
    ui: &UiConfig,
    label_column: usize,
) -> String {
    let marker = if selected { "› " } else { "  " };
    let mut prefix = if display_width(marker) <= width {
        marker.to_owned()
    } else {
        String::new()
    };

    if ui.icons {
        let with_icon = format!("{marker}{} ", kind_icon(&candidate.kind, &ui.icon_style));
        if display_width(&with_icon) <= width {
            prefix = with_icon;
        }
    }

    let label = sanitize_text(&candidate.label);
    let description = if ui.descriptions {
        sanitize_text(&candidate.description)
    } else {
        String::new()
    };
    let prefix_width = display_width(&prefix);
    let text_width = width.saturating_sub(prefix_width);
    let separator = "  ";
    let separator_width = display_width(separator);

    let (label, description) = if ui.descriptions
        && !description.is_empty()
        // Descriptions at this width tend to leave too little room for the
        // label and are less useful than a clean single-column menu.
        && text_width >= 24
    {
        let minimum_description_width = 3;
        let label_budget = label_column
            .min(text_width.saturating_sub(separator_width + minimum_description_width));
        let label = truncate_to_width(&label, label_budget);
        let label = format!(
            "{}{}",
            label,
            " ".repeat(label_budget.saturating_sub(display_width(&label)))
        );
        let remaining = text_width.saturating_sub(display_width(&label) + separator_width);
        let description = truncate_to_width(&description, remaining);
        if description.is_empty() {
            (label, String::new())
        } else {
            (label, description)
        }
    } else {
        (truncate_to_width(&label, text_width), String::new())
    };

    let mut rendered = String::new();
    rendered.push_str(&paint_fragment(
        &prefix,
        FragmentRole::Icon(candidate.kind),
        selected,
        query,
        ui,
    ));
    rendered.push_str(&paint_fragment(
        &label,
        FragmentRole::Base,
        selected,
        query,
        ui,
    ));

    if !description.is_empty() {
        rendered.push_str(&paint_fragment(
            separator,
            FragmentRole::Description,
            selected,
            query,
            ui,
        ));
        rendered.push_str(&paint_fragment(
            &description,
            FragmentRole::Description,
            selected,
            query,
            ui,
        ));
    }

    let visible_width = display_width(&prefix)
        + display_width(&label)
        + if description.is_empty() {
            0
        } else {
            separator_width + display_width(&description)
        };
    if visible_width < width {
        rendered.push_str(&paint_fragment(
            &" ".repeat(width - visible_width),
            FragmentRole::Base,
            selected,
            query,
            ui,
        ));
    }
    rendered
}

fn kind_icon(kind: &CandidateKind, style: &str) -> &'static str {
    if style == "nerd" {
        return match kind {
            CandidateKind::Command => "\u{ea85}",
            CandidateKind::Alias => "\u{eb15}",
            CandidateKind::Function => "\u{ea8c}",
            CandidateKind::Cmdlet => "\u{eb5f}",
            CandidateKind::Subcommand => "\u{ea9c}",
            CandidateKind::Option => "\u{eb52}",
            CandidateKind::File => "\u{ea7b}",
            CandidateKind::Directory => "\u{ea83}",
            CandidateKind::Value => "\u{ea90}",
        };
    }

    match kind {
        CandidateKind::Command => "⌘",
        CandidateKind::Alias => "≈",
        CandidateKind::Function => "ƒ",
        CandidateKind::Cmdlet => "◇",
        CandidateKind::Subcommand => "↳",
        CandidateKind::Option => "−",
        CandidateKind::File => "□",
        CandidateKind::Directory => "▰",
        CandidateKind::Value => "•",
    }
}

fn paint_fragment(
    text: &str,
    role: FragmentRole,
    selected: bool,
    query: &str,
    ui: &UiConfig,
) -> String {
    if text.is_empty() {
        return String::new();
    }

    let matches = if matches!(role, FragmentRole::Border) {
        Vec::new()
    } else {
        find_matches(text, query)
    };
    if matches.is_empty() {
        return paint_span(text, role, selected, ui);
    }

    let mut rendered = String::new();
    let mut cursor = 0;
    for (start, end) in matches {
        if start > cursor {
            rendered.push_str(&paint_span(&text[cursor..start], role, selected, ui));
        }
        rendered.push_str(&paint_span(
            &text[start..end],
            FragmentRole::Match,
            selected,
            ui,
        ));
        cursor = end;
    }
    if cursor < text.len() {
        rendered.push_str(&paint_span(&text[cursor..], role, selected, ui));
    }
    rendered
}

fn paint_span(text: &str, role: FragmentRole, selected: bool, ui: &UiConfig) -> String {
    if text.is_empty() || no_color() {
        return text.to_owned();
    }

    let prefix = style_prefix(role, selected, ui);
    if prefix.is_empty() {
        text.to_owned()
    } else {
        format!("{prefix}{text}\x1b[0m")
    }
}

fn style_prefix(role: FragmentRole, selected: bool, ui: &UiConfig) -> String {
    let mut codes = Vec::new();

    match role {
        FragmentRole::Icon(kind) => {
            let color = match kind {
                CandidateKind::Directory | CandidateKind::Subcommand => &ui.match_color,
                CandidateKind::File | CandidateKind::Value => &ui.description_color,
                _ => &ui.border_color,
            };
            push_color_code(
                &mut codes,
                38,
                if selected {
                    &ui.selected_foreground
                } else {
                    color
                },
            );
            push_color_code(
                &mut codes,
                48,
                if selected {
                    &ui.selected_background
                } else {
                    &ui.background
                },
            );
        }
        FragmentRole::Base => {
            push_color_code(
                &mut codes,
                38,
                if selected {
                    &ui.selected_foreground
                } else {
                    &ui.foreground
                },
            );
            push_color_code(
                &mut codes,
                48,
                if selected {
                    &ui.selected_background
                } else {
                    &ui.background
                },
            );
        }
        FragmentRole::Description => {
            push_color_code(&mut codes, 38, &ui.description_color);
            push_color_code(
                &mut codes,
                48,
                if selected {
                    &ui.selected_background
                } else {
                    &ui.background
                },
            );
        }
        FragmentRole::Match => {
            push_color_code(&mut codes, 38, &ui.match_color);
            push_color_code(
                &mut codes,
                48,
                if selected {
                    &ui.selected_background
                } else {
                    &ui.background
                },
            );
            if ui.match_color == "default" {
                codes.push("1".to_owned());
            }
        }
        FragmentRole::Border => {
            push_color_code(&mut codes, 38, &ui.border_color);
        }
    }

    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

fn push_color_code(codes: &mut Vec<String>, mode: u8, value: &str) {
    if let Some((red, green, blue)) = parse_color(value) {
        codes.push(format!("{mode};2;{red};{green};{blue}"));
    }
}

fn parse_color(value: &str) -> Option<(u8, u8, u8)> {
    if !is_valid_color(value) || value == "default" {
        return None;
    }
    Some((
        u8::from_str_radix(&value[1..3], 16).ok()?,
        u8::from_str_radix(&value[3..5], 16).ok()?,
        u8::from_str_radix(&value[5..7], 16).ok()?,
    ))
}

fn no_color() -> bool {
    env::var_os("NO_COLOR").is_some()
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn truncate_to_width(text: &str, width: usize) -> String {
    if display_width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".to_owned();
    }

    let mut result = take_to_width(text, width - 1);
    result.push('…');
    result
}

fn take_to_width(text: &str, width: usize) -> String {
    let mut result = String::new();
    let mut used = 0;
    for grapheme in UnicodeSegmentation::graphemes(text, true) {
        let grapheme_width = display_width(grapheme);
        if used + grapheme_width > width {
            break;
        }
        result.push_str(grapheme);
        used += grapheme_width;
    }
    result
}

/// Remove terminal control sequences and map remaining control characters to
/// spaces. ANSI styling is added only by this module after sanitization.
fn sanitize_text(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut sanitized = String::with_capacity(text.len());

    while let Some(character) = chars.next() {
        if character != '\x1b' {
            if character.is_control() {
                sanitized.push(' ');
            } else {
                sanitized.push(character);
            }
            continue;
        }

        match chars.next() {
            Some('[') => {
                // CSI sequences end at an ASCII byte in the final-byte range.
                for sequence_character in chars.by_ref() {
                    if sequence_character.is_ascii() && ('@'..='~').contains(&sequence_character) {
                        break;
                    }
                }
            }
            Some(']') => {
                // OSC sequences terminate with BEL or ST (ESC followed by \).
                let mut escaped = false;
                for sequence_character in chars.by_ref() {
                    if sequence_character == '\x07' {
                        break;
                    }
                    if escaped {
                        escaped = false;
                        if sequence_character == '\\' {
                            break;
                        }
                    } else if sequence_character == '\x1b' {
                        escaped = true;
                    }
                }
            }
            Some(_) | None => {}
        }
    }

    sanitized
}

/// Find non-overlapping, case-insensitive matches while retaining byte spans
/// into the original UTF-8 text for safe styling.
fn find_matches(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let folded_query = query.to_lowercase();
    if folded_query.is_empty() {
        return Vec::new();
    }

    let mut folded_text = String::new();
    let mut folded_boundary = vec![0usize];
    for (original_start, character) in text.char_indices() {
        let original_end = original_start + character.len_utf8();
        let folded_piece = character.to_lowercase().collect::<String>();
        folded_text.push_str(&folded_piece);
        while folded_boundary.len() < folded_text.len() {
            folded_boundary.push(original_start);
        }
        folded_boundary.push(original_end);
    }

    folded_text
        .match_indices(&folded_query)
        .filter_map(|(folded_start, matched)| {
            let folded_end = folded_start + matched.len();
            let original_start = *folded_boundary.get(folded_start)?;
            let original_end = *folded_boundary.get(folded_end)?;
            (original_start < original_end).then_some((original_start, original_end))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn candidate(label: &str, description: &str) -> Candidate {
        Candidate {
            label: label.to_owned(),
            insert_text: label.to_owned(),
            description: description.to_owned(),
            kind: CandidateKind::Command,
            ..Default::default()
        }
    }

    fn strip_ansi(text: &str) -> String {
        let mut output = String::new();
        let mut chars = text.chars().peekable();
        while let Some(character) = chars.next() {
            if character != '\x1b' {
                output.push(character);
                continue;
            }
            if chars.next() != Some('[') {
                continue;
            }
            for sequence_character in chars.by_ref() {
                if sequence_character.is_ascii() && ('@'..='~').contains(&sequence_character) {
                    break;
                }
            }
        }
        output
    }

    #[test]
    fn unicode_rows_are_bounded_and_have_the_declared_width() {
        let mut config = Config::default();
        config.ui.border = "none".to_owned();
        config.ui.icons = false;
        let frame = render(&[candidate("中文文件", "宽字符说明")], 0, "文", 18, &config);

        assert_eq!(frame.width, 18);
        assert_eq!(frame.lines.len(), 1);
        assert_eq!(
            display_width(&strip_ansi(&frame.lines[0])),
            frame.width as usize
        );
    }

    #[test]
    fn narrow_terminals_hide_descriptions_without_panicking() {
        let mut config = Config::default();
        config.ui.border = "rounded".to_owned();
        for width in 1..=8 {
            let frame = render(
                &[candidate(
                    "a very long label",
                    "a description that can be shortened",
                )],
                0,
                "",
                width,
                &config,
            );
            assert!(frame.width <= width);
            for line in &frame.lines {
                assert!(display_width(&strip_ansi(line)) <= frame.width as usize);
            }
        }
    }

    #[test]
    fn chinese_description_truncates_by_terminal_cells_and_can_be_hidden() {
        let mut config = Config::default();
        config.ui.icons = false;
        let item = candidate("--release", "使用发布配置构建，默认启用优化");
        for width in 32..=48 {
            let frame = render(std::slice::from_ref(&item), 0, "", width, &config);
            let visible: Vec<_> = frame.lines.iter().map(|line| strip_ansi(line)).collect();
            assert!(visible.iter().any(|line| line.contains("使用")));
            assert!(
                visible
                    .iter()
                    .all(|line| display_width(line) == usize::from(frame.width))
            );
            assert!(!visible.iter().any(|line| line.contains('�')));
        }
        config.ui.descriptions = false;
        let frame = render(&[item], 0, "", 48, &config);
        assert!(!frame.lines.join("").contains("使用"));
    }

    #[test]
    fn labels_and_descriptions_cannot_inject_escapes() {
        let mut config = Config::default();
        config.ui.border = "none".to_owned();
        let frame = render(
            &[candidate(
                "ok\x1b[31mBAD\x1b[0m\nnext",
                "desc\x1b]0;title\x07",
            )],
            0,
            "",
            60,
            &config,
        );
        let plain = strip_ansi(&frame.lines[0]);
        assert!(!plain.contains('\x1b'));
        assert!(plain.contains("okBAD"));
        assert!(plain.contains("next"));
    }

    #[test]
    fn selected_page_follows_selection_and_matches_are_styled() {
        let mut config = Config::default();
        config.ui.border = "none".to_owned();
        config.ui.max_rows = 2;
        let candidates = (0..5)
            .map(|index| candidate(&format!("item{index}"), "entry"))
            .collect::<Vec<_>>();
        let frame = render(&candidates, 4, "item", 30, &config);
        let plain = frame
            .lines
            .iter()
            .map(|line| strip_ansi(line))
            .collect::<Vec<_>>();
        assert_eq!(plain.len(), 2);
        assert!(plain.iter().any(|line| line.contains("item4")));
        if !no_color() {
            assert!(
                frame
                    .lines
                    .iter()
                    .any(|line| line.contains("38;2;255;204;102"))
            );
        }
    }

    #[test]
    fn invalid_manual_color_does_not_become_terminal_control_data() {
        let mut config = Config::default();
        config.ui.border = "none".to_owned();
        config.ui.foreground = "\x1b[31m".to_owned();
        let frame = render(&[candidate("safe", "")], 0, "", 12, &config);
        assert!(!frame.lines[0].contains("31m"));
    }

    #[test]
    fn details_and_loading_fit_small_viewports() {
        let config = Config::default();
        let mut item = candidate("--features", "启用指定功能");
        item.source = "Cargo.toml".into();
        item.detail = "格式：a,b\n示例：cargo build --features a,b".into();
        for width in [1, 8, 20, 45, 120] {
            for rows in 0..16 {
                let frame = render_with_state(
                    std::slice::from_ref(&item),
                    0,
                    "",
                    width,
                    &config,
                    MenuState {
                        incomplete: true,
                        details: true,
                        ..Default::default()
                    },
                    rows,
                );
                assert!(frame.lines.len() <= rows);
                assert!(frame.width <= width);
                assert!(
                    frame
                        .lines
                        .iter()
                        .all(|line| display_width(&strip_ansi(line)) <= usize::from(frame.width))
                );
            }
        }
        let frame = render_with_state(
            &[item],
            0,
            "",
            120,
            &config,
            MenuState {
                incomplete: true,
                details: true,
                ..Default::default()
            },
            15,
        );
        let plain = frame.lines.join("\n");
        assert!(plain.contains("正在加载"));
        assert!(plain.contains("格式：a,b"));
        let mut long_source = candidate("cargo", "管理 Rust 项目");
        long_source.source = "C:/very/long/path/".repeat(20);
        let frame = render_with_state(
            &[long_source],
            0,
            "",
            50,
            &config,
            MenuState {
                incomplete: true,
                ..Default::default()
            },
            15,
        );
        let plain = frame.lines.join("\n");
        assert!(plain.contains("正在加载"));
        assert!(plain.contains("F1 详情"));
    }
}
