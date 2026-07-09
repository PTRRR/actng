use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::view::titled_block;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let queue = app.review_queue();
    let filtered_queue: Vec<usize> = if let Some(file_idx) = app.file_filter {
        queue
            .into_iter()
            .filter(|&i| app.dataset.source[i] == file_idx)
            .collect()
    } else {
        queue
    };

    if filtered_queue.is_empty() {
        let msg = if app.review_all {
            "no entries in this dataset"
        } else {
            "nothing left to review \u{2014} press 'a' to retag everything"
        };
        frame.render_widget(Paragraph::new(msg).block(titled_block("Review")), area);
        return;
    }

    let cursor = app.review_cursor.min(filtered_queue.len() - 1);
    let entry_idx = filtered_queue[cursor];
    let entry = &app.dataset.entries[entry_idx];

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(area);

    let date = entry
        .date
        .map(|d| d.to_string())
        .unwrap_or_else(|| "?".to_string());
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
                format!(
                    "currently: {} ({:?}, {:.0}%)",
                    sugg.tag,
                    sugg.source,
                    sugg.confidence * 100.0
                ),
                Style::default().fg(Color::Cyan),
            )));
        } else {
            header_lines.push(Line::from(Span::styled(
                "currently: untagged",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(header_lines)
            .block(titled_block(format!(
                "Review \u{2014} {} of {}",
                cursor + 1,
                filtered_queue.len()
            )))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        chunks[0],
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    let candidates = app.profile.candidates(&entry.description);
    let cand_items: Vec<ListItem> = if candidates.is_empty() {
        vec![ListItem::new(
            "(no candidates \u{2014} use t to pick a tag)",
        )]
    } else {
        candidates
            .iter()
            .take(9)
            .enumerate()
            .map(|(i, (tag, conf))| ListItem::new(format!("{}  {:<16} {:.2}", i + 1, tag, conf)))
            .collect()
    };
    frame.render_widget(
        List::new(cand_items).block(titled_block("Candidates")),
        body[0],
    );

    let queue_items: Vec<ListItem> = filtered_queue
        .iter()
        .enumerate()
        .map(|(i, &idx)| {
            let e = &app.dataset.entries[idx];
            let amount = e.amount.map(|a| format!("{a:.2}")).unwrap_or_default();
            ListItem::new(format!(
                "{:<40} {:>10}",
                truncate(&e.description, 40),
                amount
            ))
        })
        .collect();
    let mut list_state = app.review_state.clone();
    list_state.select(Some(cursor));
    frame.render_stateful_widget(
        List::new(queue_items)
            .block(titled_block("Queue"))
            .highlight_symbol("\u{25b8} ")
            .highlight_style(Style::default().add_modifier(Modifier::BOLD)),
        body[1],
        &mut list_state,
    );

    let help = "1-9 confirm \u{b7} t/Enter picker \u{b7} n new tag \u{b7} s skip \u{b7} u undo \u{b7} a all-entries";
    frame.render_widget(
        Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
        chunks[2],
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "\u{2026}"
    }
}
