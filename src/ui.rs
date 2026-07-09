use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use crate::files::FileEntry;

pub struct View<'a> {
    pub path: String,
    pub rows: Vec<&'a FileEntry>,
    pub total_rows: usize,
    pub selected: usize,
    pub filter_mode: bool,
    pub query: &'a str,
    pub show_hidden: bool,
    pub pinned: bool,
    pub status: ViewStatus<'a>,
    pub listing_error: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub enum ViewStatus<'a> {
    Ready { message: Option<&'a str> },
    Reconnecting { message: &'a str },
}

pub fn draw(frame: &mut Frame<'_>, view: &View<'_>) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, chunks[0], view);
    match view.status {
        ViewStatus::Ready { .. } => draw_rows(frame, chunks[1], view),
        ViewStatus::Reconnecting { message } => draw_reconnect(frame, chunks[1], message),
    }
    draw_footer(frame, chunks[2], view);
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, view: &View<'_>) {
    let marker = if view.pinned { "● " } else { "  " };
    let width = area.width.saturating_sub(2) as usize;
    let path = middle_truncate(&view.path, width);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(marker, Style::new().add_modifier(Modifier::DIM)),
            Span::styled(path, Style::new().add_modifier(Modifier::BOLD)),
        ])),
        area,
    );
}

fn draw_rows(frame: &mut Frame<'_>, area: Rect, view: &View<'_>) {
    if area.height == 0 {
        return;
    }
    if let Some(error) = view.listing_error {
        let text = middle_truncate(error, area.width as usize);
        frame.render_widget(Paragraph::new(text), area);
        return;
    }
    if view.rows.is_empty() {
        frame.render_widget(Paragraph::new("No files"), area);
        return;
    }

    let visible_height = area.height as usize;
    let offset = scroll_offset(view.selected, visible_height, view.rows.len());
    for (line_index, entry) in view
        .rows
        .iter()
        .skip(offset)
        .take(visible_height)
        .enumerate()
    {
        let selected = offset + line_index == view.selected;
        let prefix = if entry.is_dir() { "▸ " } else { "  " };
        let name = middle_truncate(&entry.name, area.width.saturating_sub(2) as usize);
        let style = if selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, style.add_modifier(Modifier::DIM)),
                Span::styled(name, style),
            ])),
            Rect::new(area.x, area.y + line_index as u16, area.width, 1),
        );
    }
}

fn draw_reconnect(frame: &mut Frame<'_>, area: Rect, message: &str) {
    let lines = [
        "Reconnecting to cmux",
        message,
        "Standalone: CMUX_TUI_SOCKET=/path/to/socket cargo run",
    ];
    for (index, line) in lines.iter().enumerate().take(area.height as usize) {
        frame.render_widget(
            Paragraph::new(middle_truncate(line, area.width as usize)),
            Rect::new(area.x, area.y + index as u16, area.width, 1),
        );
    }
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, view: &View<'_>) {
    let text = if view.filter_mode {
        format!("/{}█", view.query)
    } else if let ViewStatus::Ready {
        message: Some(message),
    } = view.status
    {
        message.to_string()
    } else {
        format!(
            "{}/{}  .:{}  / filter",
            view.rows.len(),
            view.total_rows,
            if view.show_hidden { "on" } else { "off" }
        )
    };
    frame.render_widget(
        Paragraph::new(middle_truncate(&text, area.width as usize)),
        area,
    );
}

fn scroll_offset(selected: usize, visible_height: usize, total: usize) -> usize {
    if visible_height == 0 || total <= visible_height || selected < visible_height {
        return 0;
    }
    (selected + 1)
        .saturating_sub(visible_height)
        .min(total - visible_height)
}

pub fn middle_truncate(input: &str, max_chars: usize) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return input.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let keep = max_chars - 3;
    let front = keep.div_ceil(2);
    let back = keep / 2;
    let mut output = chars[..front].iter().collect::<String>();
    output.push_str("...");
    output.extend(&chars[chars.len() - back..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_truncates_for_narrow_columns() {
        assert_eq!(middle_truncate("abcdefghi", 7), "ab...hi");
        assert_eq!(middle_truncate("abcdefghi", 3), "...");
        assert_eq!(middle_truncate("abc", 3), "abc");
        assert_eq!(middle_truncate("abc", 0), "");
    }
}
