pub mod actions;
pub mod app;
pub mod modal;
pub mod screens;
pub mod update;
pub mod view;

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Flex, Layout};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Terminal;

use actng_core::Profile;

use crate::actions::{persist_new_layouts, resolve_profile_path, save_profile_atomically};
use crate::app::{App, Cmd, FileDetail, Modal, Msg};

const TICK_RATE: Duration = Duration::from_millis(250);

pub fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, dir: &Path, profile_path: &Path) -> anyhow::Result<()> {
    let profile = if profile_path.exists() {
        Profile::load(profile_path)?
    } else {
        match prompt_create_profile(terminal, profile_path)? {
            Some(profile) => profile,
            None => return Ok(()),
        }
    };

    let mut app = load_dataset(profile, profile_path.to_path_buf(), dir.to_path_buf())?;

    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|f| view::view(&app, f))?;

        let timeout = TICK_RATE.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            let msg = match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => Some(Msg::Key(key)),
                Event::Resize(_, _) => Some(Msg::Resize),
                _ => None,
            };
            if let Some(msg) = msg {
                let cmds = update::update(&mut app, msg);
                for cmd in cmds {
                    execute(&mut app, cmd)?;
                }
            }
        }
        if last_tick.elapsed() >= TICK_RATE {
            update::update(&mut app, Msg::Tick);
            last_tick = Instant::now();
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn load_dataset(mut profile: Profile, profile_path: PathBuf, dir: PathBuf) -> anyhow::Result<App> {
    let imports = actng_core::import_dir(&dir, &profile)?;

    let file_details: Vec<FileDetail> = imports
        .iter()
        .filter_map(|fi| {
            let imp = fi.result.as_ref().ok()?;
            Some(FileDetail {
                path: fi.path.clone(),
                entries: imp.entries.len(),
                delimiter: imp.delimiter,
                encoding: imp.encoding,
                skipped_rows: imp.skipped_rows,
                layout_remembered: profile.layouts.contains_key(&imp.fingerprint),
            })
        })
        .collect();

    persist_new_layouts(&profile_path, &mut profile, &imports)?;
    let dataset = actng_core::collect(imports);

    Ok(App::new(profile, profile_path, dir, dataset, file_details))
}

fn execute(app: &mut App, cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::SaveProfile => match save_profile_atomically(&app.profile_path, &app.profile) {
            Ok(()) => app.last_saved = Some(Instant::now()),
            Err(e) => app.set_error(e),
        },
        Cmd::WriteExport(path, want_summary) => match std::fs::File::create(&path) {
            Ok(file) => match actng_core::write_csv(file, &app.dataset, &app.profile, &app.suggestions) {
                Ok(summary) => {
                    if want_summary {
                        app.modal = Some(Modal::ExportSummary(summary.per_category));
                    } else {
                        app.set_toast(format!("exported {} rows to {}", summary.rows, path.display()));
                    }
                }
                Err(e) => app.set_error(e),
            },
            Err(e) => app.set_error(e),
        },
        Cmd::Rescan => {
            let dir = app.dir.clone();
            let profile_path = app.profile_path.clone();
            let profile = std::mem::replace(&mut app.profile, Profile::new(""));
            match load_dataset(profile, profile_path, dir) {
                Ok(new_app) => {
                    app.profile = new_app.profile;
                    app.dataset = new_app.dataset;
                    app.file_details = new_app.file_details;
                    app.recompute();
                    app.set_toast("re-scanned");
                }
                Err(e) => app.set_toast(format!("rescan failed: {e}")),
            }
        }
        Cmd::Quit => {}
    }
    Ok(())
}

fn prompt_create_profile(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    profile_path: &Path,
) -> anyhow::Result<Option<Profile>> {
    let mut name = String::new();
    loop {
        terminal.draw(|f| {
            let area = f.area();
            let [rect] = Layout::horizontal([Constraint::Length(60)]).flex(Flex::Center).areas(area);
            let [rect] = Layout::vertical([Constraint::Length(5)]).flex(Flex::Center).areas(rect);
            f.render_widget(Clear, rect);
            let text = format!(
                "No profile found at {}\n\nProfile name: {}\u{2588}\n\nEnter: create \u{b7} Esc: exit",
                profile_path.display(),
                name
            );
            f.render_widget(Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Create profile")), rect);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                crossterm::event::KeyCode::Esc => return Ok(None),
                crossterm::event::KeyCode::Enter => {
                    let name = if name.trim().is_empty() { "default".to_string() } else { name.trim().to_string() };
                    let profile = Profile::new(name);
                    save_profile_atomically(profile_path, &profile)?;
                    return Ok(Some(profile));
                }
                crossterm::event::KeyCode::Backspace => {
                    name.pop();
                }
                crossterm::event::KeyCode::Char(c) => name.push(c),
                _ => {}
            }
        }
    }
}
