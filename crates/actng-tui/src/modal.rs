use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::actions::{picker_items, PickerItem};
use crate::app::{App, ExportField, Modal};
use crate::view::titled_block;

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)]).flex(Flex::Center).areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center).areas(area);
    area
}

pub fn render(app: &App, modal: &Modal, frame: &mut Frame, area: Rect) {
    match modal {
        Modal::Help => render_help(frame, area),
        Modal::Error(msg) => render_message(frame, area, "Error", msg, Color::Red),
        Modal::Picker(picker) => render_picker(app, picker, frame, area),
        Modal::TextPrompt(prompt) => render_text_prompt(prompt, frame, area),
        Modal::Confirm(confirm) => render_message(frame, area, "Confirm (y/n)", &confirm.message, Color::Yellow),
        Modal::ExportForm(form) => render_export_form(form, frame, area),
        Modal::ExportSummary(summary) => render_export_summary(summary, frame, area),
    }
}

fn render_help(frame: &mut Frame, area: Rect) {
    let rect = centered(area, 60, 16);
    let lines = vec![
        Line::from("1-5/Tab  switch screen"),
        Line::from("e        export modal"),
        Line::from("R        re-scan directory"),
        Line::from("?        this help"),
        Line::from("Esc      close modal / back"),
        Line::from("q        quit"),
        Line::from(""),
        Line::from("Review: 1-9 confirm \u{b7} t/Enter picker \u{b7} n new tag \u{b7} x exception"),
        Line::from("        s skip \u{b7} u undo \u{b7} a toggle retag-all"),
        Line::from("Entries: / search \u{b7} f filter \u{b7} Enter retag \u{b7} x exception"),
        Line::from("Tags: a add \u{b7} r rename \u{b7} c category \u{b7} d delete \u{b7} s sort"),
        Line::from(""),
        Line::from("press any key to close"),
    ];
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(titled_block("Help")), rect);
}

fn render_message(frame: &mut Frame, area: Rect, title: &str, message: &str, color: Color) {
    let rect = centered(area, 70, 7);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(message).wrap(ratatui::widgets::Wrap { trim: true }).style(Style::default().fg(color)).block(titled_block(title)),
        rect,
    );
}

fn render_picker(app: &App, picker: &crate::app::Picker, frame: &mut Frame, area: Rect) {
    let rect = centered(area, 60, 16);
    frame.render_widget(Clear, rect);

    let items = picker_items(app, picker);
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let selected = i == picker.selected.min(items.len().saturating_sub(1));
            let style = if selected { Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD) } else { Style::default() };
            let text = match item {
                PickerItem::Tag { name, category } => match category {
                    Some(c) => format!("{name}  ({c})"),
                    None => name.clone(),
                },
                PickerItem::Category(c) => c.clone(),
                PickerItem::Create(q) => format!("create new \"{q}\""),
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let title = format!("Pick a tag > {}", picker.query);
    frame.render_widget(List::new(list_items).block(titled_block(title)), rect);
}

fn render_text_prompt(prompt: &crate::app::TextPrompt, frame: &mut Frame, area: Rect) {
    let rect = centered(area, 50, 3);
    frame.render_widget(Clear, rect);
    let title = match prompt.purpose {
        crate::app::TextPromptPurpose::AddTag => "New tag name",
        crate::app::TextPromptPurpose::RenameTag => "Rename tag to",
    };
    frame.render_widget(Paragraph::new(format!("{}\u{2588}", prompt.input)).block(titled_block(title)), rect);
}

fn render_export_form(form: &crate::app::ExportForm, frame: &mut Frame, area: Rect) {
    let rect = centered(area, 70, 6);
    frame.render_widget(Clear, rect);
    let path_style = if form.field == ExportField::Path { Style::default().fg(Color::Cyan) } else { Style::default() };
    let summary_style = if form.field == ExportField::Summary { Style::default().fg(Color::Cyan) } else { Style::default() };
    let checkbox = if form.summary { "[x]" } else { "[ ]" };
    let lines = vec![
        Line::styled(format!("path:    {}\u{2588}", form.path), path_style),
        Line::styled(format!("{checkbox} include per-category summary"), summary_style),
        Line::from(""),
        Line::from("Tab: switch field \u{b7} Space: toggle \u{b7} Enter: export \u{b7} Esc: cancel"),
    ];
    frame.render_widget(Paragraph::new(lines).block(titled_block("Export")), rect);
}

fn render_export_summary(summary: &[(String, f64)], frame: &mut Frame, area: Rect) {
    let rect = centered(area, 50, (summary.len() as u16 + 4).min(20));
    frame.render_widget(Clear, rect);
    let mut lines: Vec<Line> = summary.iter().map(|(cat, total)| Line::from(format!("{cat:<24} {total:>12.2}"))).collect();
    lines.push(Line::from(""));
    lines.push(Line::from("press any key to close"));
    frame.render_widget(Paragraph::new(lines).block(titled_block("Export summary")), rect);
}
