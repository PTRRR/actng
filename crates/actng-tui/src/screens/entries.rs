use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;
use ratatui::widgets::TableState;

use crate::app::{App, EntryFilter};
use crate::update::filtered_entry_indices;
use crate::view::titled_block;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let indices = filtered_entry_indices(app);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // Header section for full text
            Constraint::Min(3),    // Table section
            Constraint::Length(1), // Help line
        ])
        .split(area);

    // Render entry detail header
    if !indices.is_empty() {
        let cursor = app.entries_cursor.min(indices.len() - 1);
        let entry_idx = indices[cursor];
        let entry = &app.dataset.entries[entry_idx];

        let date = entry.date.map(|d| d.to_string()).unwrap_or_else(|| "?".to_string());
        let amount = entry.amount.map(|a| format!("{a:.2}")).unwrap_or_default();
        let total: f64 = indices.iter().filter_map(|&idx| app.dataset.entries[idx].amount).sum();
        let (tag_text, tag_style) = tag_display(&app.suggestions[entry_idx]);

        let header_lines = vec![
            Line::from(vec![
                Span::raw(format!("{date}   ")),
                Span::raw(format!("{amount}   ")),
                Span::raw(format!("{}", entry.description)),
            ]),
            Line::from(vec![
                Span::raw("Tag: "),
                Span::styled(tag_text, tag_style),
                Span::raw(format!("  Total: {total:.2}")),
            ]),
        ];

        frame.render_widget(
            Paragraph::new(header_lines)
                .block(titled_block(format!("Detail \u{2014} {} of {}", cursor + 1, indices.len())))
                .wrap(ratatui::widgets::Wrap { trim: false }),
            chunks[0],
        );
    } else {
        frame.render_widget(
            Paragraph::new("no entries found").block(titled_block("Detail")),
            chunks[0],
        );
    }

    let filter_label = match &app.entries_filter {
        EntryFilter::All => "all".to_string(),
        EntryFilter::Tagged => "tagged".to_string(),
        EntryFilter::Review => "review".to_string(),
        EntryFilter::Overridden => "overridden".to_string(),
        EntryFilter::Tag(t) => format!("tag:{t}"),
    };

    let header = Row::new(vec!["Date", "Amount", "Description", "Tag", "Source"])
        .style(Style::default().add_modifier(ratatui::style::Modifier::BOLD));

    let rows: Vec<Row> = indices
        .iter()
        .enumerate()
        .map(|(row_i, &idx)| {
            let e = &app.dataset.entries[idx];
            let date = e.date.map(|d| d.to_string()).unwrap_or_else(|| "?".to_string());
            let amount = e.amount.map(|a| format!("{a:.2}")).unwrap_or_default();
            let (tag_text, tag_style) = tag_display(&app.suggestions[idx]);
            let source = app.dataset.sources.get(app.dataset.source[idx]).map(|p| p.display().to_string()).unwrap_or_default();
            Row::new(vec![
                Cell::from(date),
                Cell::from(amount),
                Cell::from(e.description.clone()),
                Cell::from(tag_text).style(tag_style),
                Cell::from(source),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Min(30),
        Constraint::Length(18),
        Constraint::Percentage(20),
    ];
    let search_suffix = app.entries_search.as_deref().map(|s| format!(" /{s}")).unwrap_or_default();
    let table = Table::new(rows, widths)
        .header(header)
        .block(titled_block(format!("Entries \u{2014} {filter_label} ({}){search_suffix}", indices.len())))
        .highlight_style(Style::default().bg(Color::DarkGray));

    let mut table_state = app.entries_state.clone();
    table_state.select(Some(app.entries_cursor));
    frame.render_stateful_widget(table, chunks[1], &mut table_state);


    let help = "/ search \u{b7} f cycle filter \u{b7} Enter retag \u{b7} x exception \u{b7} j/k move";
    frame.render_widget(Paragraph::new(help).style(Style::default().fg(Color::DarkGray)), chunks[2]);
}

/// Tag text and color for a suggestion, distinguishing override / exact /
/// bayes / untagged so the Entries table doubles as an audit view.
fn tag_display(suggestion: &Option<actng_core::Suggestion>) -> (String, Style) {
    match suggestion {
        Some(s) if s.source == actng_core::Source::Override => (s.tag.clone(), Style::default().fg(Color::Magenta)),
        Some(s) if s.source == actng_core::Source::Exact => (s.tag.clone(), Style::default().fg(Color::Green)),
        Some(s) => (s.tag.clone(), Style::default().fg(Color::Yellow)),
        None => ("(review)".to_string(), Style::default().fg(Color::Red)),
    }
}
