use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::update::sorted_tags;
use crate::view::titled_block;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    let names = sorted_tags(app);
    let stats = app.profile.tagger.stats();
    let header = Row::new(vec!["Tag", "Category", "Trained", "Exact keys"]).style(Style::default().add_modifier(ratatui::style::Modifier::BOLD));

    let rows: Vec<Row> = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let category = app.profile.tags.iter().find(|t| &t.name == name).and_then(|t| t.category.clone()).unwrap_or_else(|| "\u{2014}".to_string());
            let (trained, exact_keys) = stats.iter().find(|s| &s.tag == name).map(|s| (s.trained_docs, s.exact_keys)).unwrap_or((0, 0));
            let selected = i == app.tags_cursor;
            let style = if selected { Style::default().bg(Color::DarkGray) } else { Style::default() };
            Row::new(vec![
                Cell::from(name.clone()),
                Cell::from(category),
                Cell::from(trained.to_string()),
                Cell::from(exact_keys.to_string()),
            ])
            .style(style)
        })
        .collect();

    let widths = [Constraint::Percentage(30), Constraint::Percentage(30), Constraint::Length(10), Constraint::Length(12)];
    let table = Table::new(rows, widths).header(header).block(titled_block(format!("Tags \u{2014} sort:{}", app.tags_sort.label())));
    frame.render_widget(table, chunks[0]);

    let help = "a add \u{b7} r rename \u{b7} c category \u{b7} d delete \u{b7} s cycle sort";
    frame.render_widget(Paragraph::new(help).style(Style::default().fg(Color::DarkGray)), chunks[1]);
}
