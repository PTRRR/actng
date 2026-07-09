use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Table, Row};
use ratatui::Frame;
use std::collections::HashMap;

use crate::app::App;
use crate::view::titled_block;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let block = titled_block("Summary");
    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let mut tag_totals = HashMap::new();
    for (i, entry) in app.dataset.entries.iter().enumerate() {
        if let Some(file_idx) = app.file_filter {
            if app.dataset.source[i] != file_idx {
                continue;
            }
        }
        if let Some(sugg) = &app.suggestions[i] {
            let tag = sugg.tag.clone();
            let amount = entry.amount.unwrap_or(0.0);
            let stats = tag_totals.entry(tag).or_insert((0.0, 0.0));
            if amount > 0.0 {
                stats.0 += amount;
            } else {
                stats.1 += amount.abs();
            }
        }
    }

    let mut rows = Vec::new();
    let mut sorted_tags: Vec<_> = tag_totals.into_iter().collect();
    sorted_tags.sort_by(|a, b| {
        let total_a = a.1 .0 - a.1 .1;
        let total_b = b.1 .0 - b.1 .1;
        total_b.partial_cmp(&total_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    for (tag, (credits, debits)) in sorted_tags {
        rows.push(Row::new(vec![
            Span::raw(tag),
            Span::raw(format!("{:.2}", credits)),
            Span::raw(format!("{:.2}", debits)),
        ]));
    }

    let table = Table::new(
        rows,
        [Constraint::Percentage(50), Constraint::Percentage(25), Constraint::Percentage(25)],
    )
    .header(Row::new(vec![
        Span::styled("Tag", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled("Credits", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled("Debits", Style::default().add_modifier(Modifier::BOLD)),
    ]))
    .block(Block::default().borders(Borders::NONE));

    frame.render_widget(table, inner_area);
}
