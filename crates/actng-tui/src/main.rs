mod actions;
mod app;
mod modal;
mod screens;
mod update;
mod view;

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

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut dir = PathBuf::from(".");
    let mut profile_flag: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profile" | "-p" => {
                profile_flag = args.next().map(PathBuf::from);
            }
            other => dir = PathBuf::from(other),
        }
    }

    let profile_path = resolve_profile_path(profile_flag.as_deref());

    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &dir, &profile_path);

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = io::stdout().execute(LeaveAlternateScreen);
        default_hook(info);
    }));
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, dir: &Path, profile_path: &Path) -> anyhow::Result<()> {
    let profile = if profile_path.exists() {
        Profile::load(profile_path)?
    } else {
        match prompt_create_profile(terminal, profile_path)? {
            Some(profile) => profile,
            None => return Ok(()), // declined: exit cleanly
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

/// Load the profile's dataset from `dir`: discover files, collect + dedup,
/// remember newly detected layouts, and suggest every entry.
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

/// First-run flow: the profile file doesn't exist yet. Offer to create it
/// (name input, Enter confirms) directly in the alternate screen, before the
/// main `App` (which requires an already-loaded `Profile`) is constructed.
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

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};

    use crate::app::{Msg, Screen};
    use crate::update::update;

    use super::*;

    const FIXTURE_CSV: &str = "Date,Description,Amount\n\
2025-01-01,COOP LAUSANNE,-12.50\n\
2025-01-02,MIGROS RENENS,-8.90\n\
2025-01-03,SALARY PAYMENT,2500.00\n";

    fn key(c: char) -> Msg {
        Msg::Key(crossterm::event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn keycode(code: KeyCode) -> Msg {
        Msg::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// Full headless session over a temp dir with no pre-existing profile:
    /// create the profile, import the fixture, train a couple of tags
    /// through the same `update`/`execute` path the real event loop uses,
    /// and export — then check the profile round-trips and the export CSV
    /// has the expected shape.
    #[test]
    fn end_to_end_session_creates_trains_and_exports() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("statement.csv"), FIXTURE_CSV).unwrap();
        let profile_path = dir.path().join("actng.json");

        assert!(!profile_path.exists());
        let profile = Profile::new("session-test");
        save_profile_atomically(&profile_path, &profile).unwrap();

        let mut app = load_dataset(profile, profile_path.clone(), dir.path().to_path_buf()).unwrap();
        assert_eq!(app.dataset.entries.len(), 3, "all three fixture rows should import with no dedup collisions");

        app.screen = Screen::Review;
        assert_eq!(app.review_queue().len(), 3, "nothing trained yet");

        // Train "groceries" via the tag picker, exactly as a user would:
        // open it with 't', type the name, confirm with Enter.
        for cmd in update(&mut app, keycode(KeyCode::Char('t'))) {
            execute(&mut app, cmd).unwrap();
        }
        for c in "groceries".chars() {
            for cmd in update(&mut app, key(c)) {
                execute(&mut app, cmd).unwrap();
            }
        }
        for cmd in update(&mut app, keycode(KeyCode::Enter)) {
            execute(&mut app, cmd).unwrap();
        }

        assert_eq!(app.profile.suggest("COOP LAUSANNE").unwrap().tag, "groceries");
        assert_eq!(app.review_queue().len(), 2, "the tagged entry left the queue");

        // The profile must already be on disk (every mutation saves).
        let reloaded = Profile::load(&profile_path).unwrap();
        assert_eq!(reloaded.name, "session-test");
        assert_eq!(reloaded.suggest("COOP LAUSANNE").unwrap().tag, "groceries");

        // Export and check the CSV shape: header + one row per entry,
        // including the two still-untagged rows.
        let export_path = dir.path().join("out.csv");
        execute(&mut app, Cmd::WriteExport(export_path.clone(), false)).unwrap();

        let mut reader = csv::Reader::from_path(&export_path).unwrap();
        let headers = reader.headers().unwrap().clone();
        assert_eq!(headers.iter().collect::<Vec<_>>(), vec!["date", "description", "amount", "tag", "category", "source_file"]);
        let rows: Vec<csv::StringRecord> = reader.records().collect::<Result<_, _>>().unwrap();
        assert_eq!(rows.len(), 3);
        let tagged_row = rows.iter().find(|r| r.get(1) == Some("COOP LAUSANNE")).unwrap();
        assert_eq!(tagged_row.get(3), Some("groceries"));
        let untagged_row = rows.iter().find(|r| r.get(1) == Some("SALARY PAYMENT")).unwrap();
        assert_eq!(untagged_row.get(3), Some(""), "untagged rows are kept with an empty tag");
    }
}
