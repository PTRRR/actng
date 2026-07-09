use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::view::titled_block;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let queue = app.review_queue();
    if queue.is_empty() {
        let msg = if app.review_all {
            "no entries in this dataset"
        } else {
            "nothing left to review \u{2014} press 'a' to retag everything"
        };
        frame.render_widget(Paragraph::new(msg).block(titled_block("Review")), area);
        return;
    }

    let cursor = app.review_cursor.min(queue.len() - 1);
    let entry_idx = queue[cursor];
    let entry = &app.dataset.entries[entry_idx];

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    let date = entry.date.map(|d| d.to_string()).unwrap_or_else(|| "?".to_string());
    let amount_span = match entry.amount {
        Some(a) if a < 0.0 => Span::styled(format!("{a:.2}"), Style::default().fg(Color::Red)),
        Some(a) => Span::styled(format!("{a:.2}"), Style::default().fg(Color::Green)),
        None => Span::raw("?"),
    };
    let mut header_lines = vec![Line::from(vec![
        Span::raw(format!("{date}   ")),
        amount_span,
        Span::raw(format!("   {}", entry.description)),
    ])];
    if app.review_all {
        if let Some(sugg) = &app.suggestions[entry_idx] {
            header_lines.push(Line::from(Span::styled(
                format!("currently: {} ({:?}, {:.0}%)", sugg.tag, sugg.source, sugg.confidence * 100.0),
                Style::default().fg(Color::Cyan),
            )));
        } else {
            header_lines.push(Line::from(Span::styled("currently: untagged", Style::default().fg(Color::DarkGray))));
        }
    }
    frame.render_widget(
        Paragraph::new(header_lines).block(titled_block(format!("Review \u{2014} {} of {}", cursor + 1, queue.len()))),
        chunks[0],
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    let candidates = app.profile.candidates(&entry.description);
    let cand_items: Vec<ListItem> = if candidates.is_empty() {
        vec![ListItem::new("(no candidates \u{2014} use t to pick a tag)")]
    } else {
        candidates
            .iter()
            .take(9)
            .enumerate()
            .map(|(i, (tag, conf))| ListItem::new(format!("{}  {:<16} {:.2}", i + 1, tag, conf)))
            .collect()
    };
    frame.render_widget(List::new(cand_items).block(titled_block("Candidates")), body[0]);

    let queue_items: Vec<ListItem> = queue
        .iter()
        .enumerate()
        .map(|(i, &idx)| {
            let e = &app.dataset.entries[idx];
            let marker = if i == cursor { "\u{25b8} " } else { "  " };
            let amount = e.amount.map(|a| format!("{a:.2}")).unwrap_or_default();
            let style = if i == cursor { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
            ListItem::new(format!("{marker}{:<40} {:>10}", truncate(&e.description, 40), amount)).style(style)
        })
        .collect();
    frame.render_widget(List::new(queue_items).block(titled_block("Queue")), body[1]);

    let help = "1-9 confirm \u{b7} t/Enter picker \u{b7} n new tag \u{b7} s skip \u{b7} u undo \u{b7} a all-entries";
    frame.render_widget(Paragraph::new(help).style(Style::default().fg(Color::DarkGray)), chunks[2]);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "\u{2026}"
    }
}
