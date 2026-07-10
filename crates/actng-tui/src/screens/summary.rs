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
    let mut cat_totals = HashMap::new();
    for (i, entry) in app.dataset.entries.iter().enumerate() {
        if let Some(file_idx) = app.file_filter {
            if app.dataset.source[i] != file_idx {
                continue;
            }
        }
        if let Some(sugg) = &app.suggestions[i] {
            let amount = entry.amount.unwrap_or(0.0);
            
            // Tag aggregation
            let tag_stats = tag_totals.entry(sugg.tag.clone()).or_insert((0.0, 0.0));
            if amount > 0.0 { tag_stats.0 += amount; } else { tag_stats.1 += amount.abs(); }

            // Category aggregation
            let category = app.profile.tags.iter()
                .find(|t| t.name == sugg.tag)
                .and_then(|t| t.category.as_deref())
                .unwrap_or("uncategorized");
            let cat_stats = cat_totals.entry(category.to_string()).or_insert((0.0, 0.0));
            if amount > 0.0 { cat_stats.0 += amount; } else { cat_stats.1 += amount.abs(); }
        }
    }

    let mut rows = Vec::new();
    
    // Tags section
    let mut sorted_tags: Vec<_> = tag_totals.into_iter().collect();
    sorted_tags.sort_by(|a, b| (b.1 .0 - b.1 .1).partial_cmp(&(a.1 .0 - a.1 .1)).unwrap_or(std::cmp::Ordering::Equal));
    
    rows.push(Row::new(vec![Span::styled("TAGS", Style::default().add_modifier(Modifier::BOLD))]).style(Style::default().fg(Color::Yellow)));
    for (tag, (credits, debits)) in sorted_tags {
        rows.push(Row::new(vec![
            Span::raw(format!("  {}", tag)),
            Span::raw(format!("{:.2}", credits)),
            Span::raw(format!("{:.2}", debits)),
        ]));
    }

    // Categories section
    let mut sorted_cats: Vec<_> = cat_totals.into_iter().collect();
    sorted_cats.sort_by(|a, b| (b.1 .0 - b.1 .1).partial_cmp(&(a.1 .0 - a.1 .1)).unwrap_or(std::cmp::Ordering::Equal));
    
    rows.push(Row::new(vec![Span::styled("CATEGORIES", Style::default().add_modifier(Modifier::BOLD))]).style(Style::default().fg(Color::Cyan)));
    for (cat, (credits, debits)) in sorted_cats {
        rows.push(Row::new(vec![
            Span::raw(format!("  {}", cat)),
            Span::raw(format!("{:.2}", credits)),
            Span::raw(format!("{:.2}", debits)),
        ]));
    }

    let table = Table::new(
        rows,
        [Constraint::Percentage(50), Constraint::Percentage(25), Constraint::Percentage(25)],
    )
    .header(Row::new(vec![
        Span::styled("Label", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled("Credits", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled("Debits", Style::default().add_modifier(Modifier::BOLD)),
    ]))
    .block(Block::default().borders(Borders::NONE));

    frame.render_widget(table, inner_area);
}
