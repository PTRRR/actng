use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Cell, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::view::titled_block;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let header = Row::new(vec!["File", "Entries", "Delim", "Encoding", "Layout", "Status"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let mut rows = Vec::new();
    for (i, d) in app.file_details.iter().enumerate() {
        let selected = i == app.files_cursor;
        let style = if selected { Style::default().bg(Color::DarkGray) } else { Style::default() };
        let provenance = if d.layout_remembered { "remembered" } else { "detected" };
        rows.push(
            Row::new(vec![
                Cell::from(d.path.display().to_string()),
                Cell::from(d.entries.to_string()),
                Cell::from((d.delimiter as char).to_string()),
                Cell::from(format!("{:?}", d.encoding)),
                Cell::from(provenance),
                Cell::from(format!("ok ({} skipped)", d.skipped_rows)),
            ])
            .style(style),
        );
    }
    for (i, (path, err)) in app.dataset.failures.iter().enumerate() {
        let selected = app.file_details.len() + i == app.files_cursor;
        let style = if selected { Style::default().bg(Color::DarkGray) } else { Style::default() }.fg(Color::Red);
        rows.push(
            Row::new(vec![
                Cell::from(path.display().to_string()),
                Cell::from("-"),
                Cell::from("-"),
                Cell::from("-"),
                Cell::from("-"),
                Cell::from(format!("FAILED: {err}")),
            ])
            .style(style),
        );
    }

    if rows.is_empty() {
        rows.push(Row::new(vec!["no files discovered", "", "", "", "", ""]));
    }

    let widths = [
        Constraint::Percentage(35),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(12),
        Constraint::Length(11),
        Constraint::Min(20),
    ];
    let table = Table::new(rows, widths).header(header).block(titled_block("Files (Enter: detail on failures)"));
    frame.render_widget(table, area);
}
