use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::view::titled_block;

pub fn render(app: &App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    render_profile_card(app, frame, chunks[0]);
    render_files(app, frame, chunks[1]);
}

fn render_profile_card(app: &App, frame: &mut Frame, area: Rect) {
    let (exact, bayes, review) = app.tagged_count();
    let trained_keys: usize = app.profile.tagger.stats().iter().map(|s| s.exact_keys).sum();
    let lines = vec![
        Line::from(format!("name:     {}", app.profile.name)),
        Line::from(format!("version:  {}", app.profile.version)),
        Line::from(format!("tags:     {}", app.profile.tags.len())),
        Line::from(format!("layouts:  {} remembered", app.profile.layouts.len())),
        Line::from(""),
        Line::from(format!("entries:  {}", app.dataset.entries.len())),
        Line::from(format!("  exact:    {exact}")),
        Line::from(format!("  bayes:    {bayes}")),
        Line::from(format!("  review:   {review}")),
        Line::from(format!("duplicates dropped: {}", app.dataset.duplicates_dropped)),
        Line::from(format!("exact-match keys trained: {trained_keys}")),
        Line::from(""),
        Line::from("press 2 to review \u{b7} e to export"),
    ];
    frame.render_widget(Paragraph::new(lines).block(titled_block("Profile")), area);
}

fn render_files(app: &App, frame: &mut Frame, area: Rect) {
    let mut items: Vec<ListItem> = app
        .dataset
        .sources
        .iter()
        .enumerate()
        .map(|(idx, path)| {
            let count = app.dataset.source.iter().filter(|&&s| s == idx).count();
            ListItem::new(format!("{}  ({} entries)", path.display(), count))
        })
        .collect();
    for (path, err) in &app.dataset.failures {
        items.push(ListItem::new(format!("{}  FAILED: {}", path.display(), err)).style(Style::default().fg(Color::Red)));
    }
    if items.is_empty() {
        items.push(ListItem::new("no bank files discovered"));
    }
    frame.render_widget(List::new(items).block(titled_block("Files")), area);
}
