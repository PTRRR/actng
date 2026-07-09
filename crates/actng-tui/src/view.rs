use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, Screen};
use crate::modal;
use crate::screens;

pub fn view(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.width < 80 || area.height < 24 {
        let msg = Paragraph::new("terminal too small (need at least 80x24)").centered();
        frame.render_widget(msg, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    frame.render_widget(Paragraph::new(screen_tabs_line(app)), chunks[0]);

    match app.screen {
        Screen::Overview => screens::overview::render(app, frame, chunks[1]),
        Screen::Review => screens::review::render(app, frame, chunks[1]),
        Screen::Entries => screens::entries::render(app, frame, chunks[1]),
        Screen::Tags => screens::tags::render(app, frame, chunks[1]),
        Screen::Files => screens::files::render(app, frame, chunks[1]),
    }

    render_status_bar(app, frame, chunks[2]);

    if let Some(m) = &app.modal {
        modal::render(app, m, frame, area);
    }
}

fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    let (exact, bayes, review) = app.tagged_count();
    let save_tick = if app.last_saved.is_some() { "\u{2713} saved" } else { "" };

    let mut spans = vec![
        Span::raw(format!(" {} ", app.profile.name)),
        Span::raw("\u{2502} "),
        Span::raw(format!("{} tags ", app.profile.tags.len())),
        Span::raw("\u{2502} "),
        Span::styled(
            format!("tagged {} ({} exact, {} bayes) \u{b7} review {} ", exact + bayes, exact, bayes, review),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw("\u{2502} "),
        Span::styled(save_tick, Style::default().fg(Color::Green)),
    ];

    if let Some(toast) = &app.toast {
        spans.push(Span::raw(" \u{2502} "));
        spans.push(Span::styled(toast.message.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    }

    let right = "?:help  q:quit";
    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(bar, area);

    let right_width = right.len() as u16;
    if area.width > right_width {
        let right_area = Rect { x: area.x + area.width - right_width - 1, y: area.y, width: right_width, height: 1 };
        frame.render_widget(
            Paragraph::new(right).style(Style::default().bg(Color::DarkGray).fg(Color::Gray)),
            right_area,
        );
    }
}

pub fn screen_tabs_line(app: &App) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, s) in Screen::ALL.iter().enumerate() {
        let style = if *s == app.screen {
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(format!(" {} {} ", i + 1, s.title()), style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

pub fn titled_block(title: impl Into<String>) -> Block<'static> {
    Block::default().borders(Borders::ALL).title(title.into())
}
