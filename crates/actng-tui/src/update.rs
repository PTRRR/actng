use crossterm::event::{KeyCode, KeyEvent};
use std::time::Duration;

use crate::actions::{self, picker_items};
use crate::app::{
    App, Cmd, Confirm, ConfirmContext, EntryFilter, ExportField, ExportForm, Modal, Msg, Picker,
    PickerContext, Screen, TagSort, TextPrompt, TextPromptPurpose,
};

const TOAST_LIFETIME: Duration = Duration::from_secs(3);

pub fn update(app: &mut App, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Resize => vec![],
        Msg::Tick => {
            if let Some(toast) = &app.toast {
                if toast.created.elapsed() > TOAST_LIFETIME {
                    app.toast = None;
                }
            }
            vec![]
        }
        Msg::Key(key) => handle_key(app, key),
    }
}

fn handle_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    if app.modal.is_some() {
        return handle_modal_key(app, key);
    }
    // The Entries incremental search box captures every key itself (so a
    // query containing 'q'/'e'/'R'/digits doesn't quit, export, rescan, or
    // switch screens mid-type); it's inline app state rather than a modal,
    // so it needs the same early-return the modal check gets above.
    if app.screen == Screen::Entries && app.entries_search.is_some() {
        return handle_entries_key(app, key);
    }

    match key.code {
        KeyCode::Char('q') => {
            app.should_quit = true;
            return vec![Cmd::Quit];
        }
        KeyCode::Char('?') => {
            app.modal = Some(Modal::Help);
            return vec![];
        }
        KeyCode::Char('e') => {
            let default_path = app.dir.join("actng-export.csv");
            app.modal = Some(Modal::ExportForm(ExportForm {
                path: default_path.display().to_string(),
                summary: true,
                field: ExportField::Path,
            }));
            return vec![];
        }
        KeyCode::Char('R') => return vec![Cmd::Rescan],
        // On the Review screen, 1-9 are the candidate-confirm shortcuts
        // (§4.2) and take priority over screen-switching; Tab still works.
        KeyCode::Tab => {
            let idx = Screen::ALL
                .iter()
                .position(|s| *s == app.screen)
                .unwrap_or(0);
            app.screen = Screen::ALL[(idx + 1) % Screen::ALL.len()];
        }
        _ => {}
    }

    match app.screen {
        Screen::Overview => vec![],
        Screen::Review => handle_review_key(app, key),
        Screen::Entries => handle_entries_key(app, key),
        Screen::Tags => handle_tags_key(app, key),
        Screen::Files => handle_files_key(app, key),
    }
}

fn handle_review_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    // 'a' (retag mode) and 'u' (undo) work even when the current queue is
    // empty — that's exactly when you'd reach for either of them — so they
    // must be handled before the empty-queue early return below.
    match key.code {
        KeyCode::Char('a') => {
            app.review_all = !app.review_all;
            app.review_cursor = 0;
            return vec![];
        }
        KeyCode::Char('u') => {
            if let Some(entry_idx) = actions::undo(app) {
                if let Some(pos) = app.review_queue().iter().position(|&i| i == entry_idx) {
                    app.review_cursor = pos;
                }
                app.set_toast("undone");
                return vec![Cmd::SaveProfile];
            }
            return vec![];
        }
        _ => {}
    }

    let queue = app.review_queue();
    if queue.is_empty() {
        return vec![];
    }
    app.review_cursor = app.review_cursor.min(queue.len() - 1);

    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.review_cursor = (app.review_cursor + 1).min(queue.len() - 1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.review_cursor = app.review_cursor.saturating_sub(1);
        }
        KeyCode::Char('s') => {
            app.review_cursor = (app.review_cursor + 1) % queue.len();
        }
        KeyCode::Char('t') | KeyCode::Char('n') | KeyCode::Enter => {
            let entry_idx = queue[app.review_cursor];
            app.modal = Some(Modal::Picker(Picker::new(PickerContext::Tag { entry_idx })));
        }
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            let n = c.to_digit(10).unwrap() as usize;
            let entry_idx = queue[app.review_cursor];
            let description = app.dataset.entries[entry_idx].description.clone();
            let candidates = app.profile.candidates(&description);
            if let Some((tag, _)) = candidates.get(n - 1) {
                return confirm_and_report(app, entry_idx, &tag.clone());
            }
        }
        _ => {}
    }
    vec![]
}

fn handle_entries_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    // While the incremental search box is open, every key feeds it first
    // (Esc/Enter close it) so letters like 'f'/'j'/'k' type into the query
    // instead of triggering the filter-cycle/movement shortcuts below.
    if app.entries_search.is_some() {
        match key.code {
            KeyCode::Esc => app.entries_search = None,
            KeyCode::Enter => app.entries_search = None,
            KeyCode::Backspace => {
                if let Some(s) = &mut app.entries_search {
                    s.pop();
                }
                app.entries_cursor = 0;
            }
            KeyCode::Char(c) => {
                if let Some(s) = &mut app.entries_search {
                    s.push(c);
                }
                app.entries_cursor = 0;
            }
            _ => {}
        }
        return vec![];
    }

    let indices = filtered_entry_indices(app);
    if !indices.is_empty() {
        app.entries_cursor = app.entries_cursor.min(indices.len() - 1);
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down if !indices.is_empty() => {
            app.entries_cursor = (app.entries_cursor + 1).min(indices.len() - 1);
        }
        KeyCode::Char('k') | KeyCode::Up if !indices.is_empty() => {
            app.entries_cursor = app.entries_cursor.saturating_sub(1);
        }
        KeyCode::Char('f') => {
            app.entries_filter = next_entry_filter(app);
            app.entries_cursor = 0;
        }
        KeyCode::Char('/') => {
            app.entries_search = Some(String::new());
        }
        KeyCode::Enter if !indices.is_empty() => {
            let entry_idx = indices[app.entries_cursor];
            app.modal = Some(Modal::Picker(Picker::new(PickerContext::Tag { entry_idx })));
        }
        _ => {}
    }
    vec![]
}

fn next_entry_filter(app: &App) -> EntryFilter {
    let tags: Vec<String> = app.profile.tags.iter().map(|t| t.name.clone()).collect();
    match &app.entries_filter {
        EntryFilter::All => EntryFilter::Tagged,
        EntryFilter::Tagged => EntryFilter::Review,
        EntryFilter::Review => tags
            .first()
            .cloned()
            .map(EntryFilter::Tag)
            .unwrap_or(EntryFilter::All),
        EntryFilter::Tag(current) => {
            let pos = tags.iter().position(|t| t == current);
            match pos.and_then(|p| tags.get(p + 1)) {
                Some(next) => EntryFilter::Tag(next.clone()),
                None => EntryFilter::All,
            }
        }
    }
}

pub fn filtered_entry_indices(app: &App) -> Vec<usize> {
    let search = app.entries_search.as_deref().unwrap_or("").to_lowercase();
    (0..app.dataset.entries.len())
        .filter(|&i| match &app.entries_filter {
            EntryFilter::All => true,
            EntryFilter::Tagged => app.suggestions[i].is_some(),
            EntryFilter::Review => app.suggestions[i].is_none(),
            EntryFilter::Tag(tag) => app.suggestions[i].as_ref().is_some_and(|s| &s.tag == tag),
        })
        .filter(|&i| {
            search.is_empty()
                || app.dataset.entries[i]
                    .description
                    .to_lowercase()
                    .contains(&search)
        })
        .collect()
}

fn handle_tags_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    let tags = sorted_tags(app);
    if !tags.is_empty() {
        app.tags_cursor = app.tags_cursor.min(tags.len() - 1);
    }

    match key.code {
        KeyCode::Char('j') | KeyCode::Down if !tags.is_empty() => {
            app.tags_cursor = (app.tags_cursor + 1).min(tags.len() - 1);
        }
        KeyCode::Char('k') | KeyCode::Up if !tags.is_empty() => {
            app.tags_cursor = app.tags_cursor.saturating_sub(1);
        }
        KeyCode::Char('s') => app.tags_sort = app.tags_sort.next(),
        KeyCode::Char('a') => {
            app.modal = Some(Modal::TextPrompt(TextPrompt {
                purpose: TextPromptPurpose::AddTag,
                input: String::new(),
                context: None,
            }));
        }
        KeyCode::Char('r') if !tags.is_empty() => {
            let current = tags[app.tags_cursor].clone();
            app.modal = Some(Modal::TextPrompt(TextPrompt {
                purpose: TextPromptPurpose::RenameTag,
                input: current.clone(),
                context: Some(current),
            }));
        }
        KeyCode::Char('c') if !tags.is_empty() => {
            let tag = tags[app.tags_cursor].clone();
            app.modal = Some(Modal::Picker(Picker::new(PickerContext::Category { tag })));
        }
        KeyCode::Char('d') if !tags.is_empty() => {
            let tag = tags[app.tags_cursor].clone();
            let trained = app
                .profile
                .tagger
                .stats()
                .into_iter()
                .find(|s| s.tag == tag)
                .map(|s| s.trained_docs)
                .unwrap_or(0);
            let plural = if trained == 1 { "" } else { "s" };
            app.modal = Some(Modal::Confirm(Confirm {
                message: format!("Delete '{tag}' and its {trained} trained document{plural}? This cannot be undone."),
                context: ConfirmContext::DeleteTag(tag),
            }));
        }
        _ => {}
    }
    vec![]
}

pub fn sorted_tags(app: &App) -> Vec<String> {
    let mut stats = app.profile.tagger.stats();
    let by_name: std::collections::HashMap<&str, _> = app
        .profile
        .tags
        .iter()
        .map(|t| (t.name.as_str(), t.category.clone()))
        .collect();
    // Declared tags with no training data yet don't show up in `stats()`; add them with zero counts.
    for tag in &app.profile.tags {
        if !stats.iter().any(|s| s.tag == tag.name) {
            stats.push(actng_core::TagStats {
                tag: tag.name.clone(),
                trained_docs: 0,
                exact_keys: 0,
            });
        }
    }
    match app.tags_sort {
        TagSort::Name => stats.sort_by(|a, b| a.tag.cmp(&b.tag)),
        TagSort::Category => stats.sort_by(|a, b| {
            let ca = by_name.get(a.tag.as_str()).cloned().flatten();
            let cb = by_name.get(b.tag.as_str()).cloned().flatten();
            ca.cmp(&cb).then_with(|| a.tag.cmp(&b.tag))
        }),
        TagSort::Trained => stats.sort_by(|a, b| {
            b.trained_docs
                .cmp(&a.trained_docs)
                .then_with(|| a.tag.cmp(&b.tag))
        }),
    }
    stats.into_iter().map(|s| s.tag).collect()
}

fn handle_files_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    let total = app.file_details.len() + app.dataset.failures.len();
    if total == 0 {
        return vec![];
    }
    app.files_cursor = app.files_cursor.min(total - 1);
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            app.files_cursor = (app.files_cursor + 1).min(total - 1)
        }
        KeyCode::Char('k') | KeyCode::Up => app.files_cursor = app.files_cursor.saturating_sub(1),
        KeyCode::Enter if app.files_cursor >= app.file_details.len() => {
            let (_, err) = &app.dataset.failures[app.files_cursor - app.file_details.len()];
            app.modal = Some(Modal::Error(err.to_string()));
        }
        _ => {}
    }
    vec![]
}

fn confirm_and_report(app: &mut App, entry_idx: usize, tag: &str) -> Vec<Cmd> {
    let before = app.review_queue();
    let Some(rec) = actions::confirm_tag(app, entry_idx, tag) else {
        return vec![];
    };
    let after = app.review_queue();
    let resolved = before
        .iter()
        .filter(|&&i| i != entry_idx && !after.contains(&i))
        .count();
    app.undo.push(rec);
    if resolved > 0 {
        app.set_toast(format!("{tag} \u{2713} (+{resolved} resolved)"));
    } else {
        app.set_toast(format!("{tag} \u{2713}"));
    }
    vec![Cmd::SaveProfile]
}

fn handle_modal_key(app: &mut App, key: KeyEvent) -> Vec<Cmd> {
    let Some(modal) = app.modal.take() else {
        return vec![];
    };
    match modal {
        Modal::Help | Modal::Error(_) | Modal::ExportSummary(_) => {
            // Any key dismisses.
            return vec![];
        }
        Modal::Picker(mut picker) => match key.code {
            KeyCode::Esc => return vec![],
            KeyCode::Enter => {
                let items = picker_items(app, &picker);
                let idx = picker.selected.min(items.len().saturating_sub(1));
                if let Some(item) = items.get(idx) {
                    return apply_picker_choice(app, &picker.context, item.label().to_string());
                }
            }
            KeyCode::Down => {
                let len = picker_items(app, &picker).len();
                if len > 0 {
                    picker.selected = (picker.selected + 1).min(len - 1);
                }
                app.modal = Some(Modal::Picker(picker));
            }
            KeyCode::Up => {
                picker.selected = picker.selected.saturating_sub(1);
                app.modal = Some(Modal::Picker(picker));
            }
            KeyCode::Backspace => {
                picker.query.pop();
                picker.selected = 0;
                app.modal = Some(Modal::Picker(picker));
            }
            KeyCode::Char(c) => {
                picker.query.push(c);
                picker.selected = 0;
                app.modal = Some(Modal::Picker(picker));
            }
            _ => {
                app.modal = Some(Modal::Picker(picker));
            }
        },
        Modal::TextPrompt(mut prompt) => match key.code {
            KeyCode::Esc => {}
            KeyCode::Enter => {
                return apply_text_prompt(app, &prompt);
            }
            KeyCode::Backspace => {
                prompt.input.pop();
                app.modal = Some(Modal::TextPrompt(prompt));
            }
            KeyCode::Char(c) => {
                prompt.input.push(c);
                app.modal = Some(Modal::TextPrompt(prompt));
            }
            _ => app.modal = Some(Modal::TextPrompt(prompt)),
        },
        Modal::Confirm(confirm) => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                return apply_confirm(app, &confirm.context);
            }
            KeyCode::Char('n') | KeyCode::Esc => {}
            _ => app.modal = Some(Modal::Confirm(confirm)),
        },
        Modal::ExportForm(mut form) => match key.code {
            KeyCode::Esc => {}
            KeyCode::Tab => {
                form.field = match form.field {
                    ExportField::Path => ExportField::Summary,
                    ExportField::Summary => ExportField::Path,
                };
                app.modal = Some(Modal::ExportForm(form));
            }
            KeyCode::Char(' ') if form.field == ExportField::Summary => {
                form.summary = !form.summary;
                app.modal = Some(Modal::ExportForm(form));
            }
            KeyCode::Enter => {
                let path = std::path::PathBuf::from(form.path.clone());
                return vec![Cmd::WriteExport(path, form.summary)];
            }
            KeyCode::Backspace if form.field == ExportField::Path => {
                form.path.pop();
                app.modal = Some(Modal::ExportForm(form));
            }
            KeyCode::Char(c) if form.field == ExportField::Path => {
                form.path.push(c);
                app.modal = Some(Modal::ExportForm(form));
            }
            _ => app.modal = Some(Modal::ExportForm(form)),
        },
    }
    vec![]
}

fn apply_picker_choice(app: &mut App, context: &PickerContext, name: String) -> Vec<Cmd> {
    match context {
        PickerContext::Tag { entry_idx } => confirm_and_report(app, *entry_idx, &name),
        PickerContext::Category { tag } => match app.profile.set_category(tag, name) {
            Ok(()) => vec![Cmd::SaveProfile],
            Err(e) => {
                app.set_error(e);
                vec![]
            }
        },
    }
}

fn apply_text_prompt(app: &mut App, prompt: &TextPrompt) -> Vec<Cmd> {
    let input = prompt.input.trim().to_string();
    if input.is_empty() {
        return vec![];
    }
    match prompt.purpose {
        TextPromptPurpose::AddTag => {
            app.profile.add_tag(input);
            vec![Cmd::SaveProfile]
        }
        TextPromptPurpose::RenameTag => {
            let old = prompt.context.clone().unwrap_or_default();
            match app.profile.rename_tag(&old, &input) {
                Ok(()) => {
                    app.recompute();
                    vec![Cmd::SaveProfile]
                }
                Err(e) => {
                    app.set_error(e);
                    vec![]
                }
            }
        }
    }
}

fn apply_confirm(app: &mut App, context: &ConfirmContext) -> Vec<Cmd> {
    match context {
        ConfirmContext::DeleteTag(tag) => {
            app.profile.remove_tag(tag);
            app.recompute();
            vec![Cmd::SaveProfile]
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use actng_core::{Dataset, Entry, Profile};
    use crossterm::event::KeyModifiers;

    use super::*;

    fn entry(desc: &str, amount: f64) -> Entry {
        Entry { date: None, description: desc.to_string(), amount: Some(amount), raw: vec![] }
    }

    fn test_app(entries: Vec<Entry>) -> App {
        let n = entries.len();
        let dataset = Dataset {
            entries,
            source: vec![0; n],
            sources: vec![PathBuf::from("test.csv")],
            failures: vec![],
            duplicates_dropped: 0,
        };
        App::new(Profile::new("test"), PathBuf::from("test-profile.json"), PathBuf::from("."), dataset, vec![])
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn keycode(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Two tags trained on a single token each, both shared with the entry
    /// under test: the classifier is deliberately kept undecided (~50/50,
    /// below the confirm threshold) so the entry stays in the review queue
    /// and `candidates()` returns a real ranked (if tied) list to confirm
    /// from — unlike a single trained tag, which the Bayes model always
    /// scores at confidence 1.0 (no competing class to weigh against).
    fn train_two_ambiguous_tags(app: &mut App) {
        app.profile.learn("alpha bravo", "climbing");
        app.profile.learn("alpha charlie", "groceries");
        app.recompute();
    }

    #[test]
    fn digit_confirms_top_candidate_and_advances_queue() {
        let mut app = test_app(vec![entry("alpha", -10.0)]);
        train_two_ambiguous_tags(&mut app);
        app.screen = Screen::Review;

        let queue = app.review_queue();
        assert_eq!(queue.len(), 1, "ambiguous entry needs review, not a confident guess");
        let entry_idx = queue[0];
        let top_tag = app.profile.candidates("alpha")[0].0.clone();

        let cmds = update(&mut app, Msg::Key(key('1')));
        assert!(matches!(cmds.as_slice(), [Cmd::SaveProfile]));
        assert_eq!(app.profile.suggest("alpha").unwrap().tag, top_tag);
        assert!(!app.review_queue().contains(&entry_idx), "confirmed entry leaves the queue");
        assert_eq!(app.undo.len(), 1);
    }

    #[test]
    fn digits_confirm_in_review_but_switch_screens_elsewhere() {
        let mut app = test_app(vec![entry("COOP LAUSANNE", -1.0)]);
        app.screen = Screen::Tags;
        update(&mut app, Msg::Key(key('2')));
        assert_eq!(app.screen, Screen::Review, "outside Review, digits still switch screens");

        // Now inside Review, '1' with no candidates is a no-op, not a screen switch.
        update(&mut app, Msg::Key(key('1')));
        assert_eq!(app.screen, Screen::Review, "inside Review, digits are confirm shortcuts, not switches");
    }

    #[test]
    fn skip_moves_cursor_without_learning() {
        let mut app = test_app(vec![entry("A MERCHANT", -1.0), entry("B MERCHANT", -2.0)]);
        app.screen = Screen::Review;
        update(&mut app, Msg::Key(key('s')));
        assert_eq!(app.review_cursor, 1);
        assert!(app.profile.tags.is_empty(), "skip must not train anything");
    }

    #[test]
    fn undo_reverses_a_confirmation() {
        let mut app = test_app(vec![entry("alpha", -10.0)]);
        train_two_ambiguous_tags(&mut app);
        app.screen = Screen::Review;

        update(&mut app, Msg::Key(key('1')));
        assert!(app.profile.suggest("alpha").is_some(), "confirming should have produced an exact match");

        let cmds = update(&mut app, Msg::Key(key('u')));
        assert!(matches!(cmds.as_slice(), [Cmd::SaveProfile]));
        assert!(app.profile.suggest("alpha").is_none(), "undo removes the exact match again");
        assert!(app.undo.is_empty());
    }

    #[test]
    fn confirming_a_merchant_auto_resolves_siblings_with_the_same_key() {
        let mut app =
            test_app(vec![entry("ACHAT TWINT DU 01.01.2025 GRIMPER.CH LAUSANNE", -10.0), entry("ACHAT TWINT DU 02.02.2025 GRIMPER.CH LAUSANNE", -12.0)]);
        app.profile.learn("SOME OTHER BOOTSTRAP TOKEN CLIMBING", "climbing");
        app.recompute();
        app.screen = Screen::Review;
        assert_eq!(app.review_queue().len(), 2);

        let entry_idx = app.review_queue()[0];
        let desc = app.dataset.entries[entry_idx].description.clone();
        let before = app.review_queue();
        let rec = actions::confirm_tag(&mut app, entry_idx, "climbing").unwrap();
        let after = app.review_queue();
        let resolved = before.iter().filter(|&&i| i != entry_idx && !after.contains(&i)).count();

        assert_eq!(rec.description, desc);
        assert_eq!(resolved, 1, "the sibling with the same normalized key should resolve too");
        assert_eq!(after.len(), 0);
    }

    #[test]
    fn retag_mode_toggle_includes_confidently_tagged_entries() {
        let mut app = test_app(vec![entry("COOP LAUSANNE", -1.0)]);
        app.profile.learn("COOP LAUSANNE", "groceries");
        app.recompute();
        app.screen = Screen::Review;
        assert!(app.review_queue().is_empty(), "already confidently tagged");

        update(&mut app, Msg::Key(key('a')));
        assert!(app.review_all);
        assert_eq!(app.review_queue().len(), 1, "retag mode surfaces every entry");
    }

    #[test]
    fn entries_filter_cycles_through_all_tagged_review_and_per_tag() {
        let mut app = test_app(vec![entry("COOP LAUSANNE", -1.0), entry("UNKNOWN MERCHANT", -2.0)]);
        app.profile.learn("COOP LAUSANNE", "groceries");
        app.recompute();
        app.screen = Screen::Entries;

        assert_eq!(app.entries_filter, EntryFilter::All);
        update(&mut app, Msg::Key(key('f')));
        assert_eq!(app.entries_filter, EntryFilter::Tagged);
        update(&mut app, Msg::Key(key('f')));
        assert_eq!(app.entries_filter, EntryFilter::Review);
        update(&mut app, Msg::Key(key('f')));
        assert_eq!(app.entries_filter, EntryFilter::Tag("groceries".to_string()));
        update(&mut app, Msg::Key(key('f')));
        assert_eq!(app.entries_filter, EntryFilter::All);
    }

    #[test]
    fn entries_search_captures_keys_that_would_otherwise_be_global_shortcuts() {
        let mut app = test_app(vec![entry("COOP LAUSANNE", -1.0)]);
        app.screen = Screen::Entries;

        update(&mut app, Msg::Key(key('/')));
        assert_eq!(app.entries_search, Some(String::new()));

        // 'q' would normally quit; while searching it must be typed instead.
        let cmds = update(&mut app, Msg::Key(key('q')));
        assert!(cmds.is_empty());
        assert!(!app.should_quit);
        assert_eq!(app.entries_search.as_deref(), Some("q"));

        update(&mut app, Msg::Key(keycode(KeyCode::Esc)));
        assert_eq!(app.entries_search, None);
    }

    #[test]
    fn picker_create_new_tag_confirms_and_learns() {
        let mut app = test_app(vec![entry("SOME NEW MERCHANT", -1.0)]);
        app.screen = Screen::Review;
        update(&mut app, Msg::Key(keycode(KeyCode::Char('t'))));
        assert!(matches!(app.modal, Some(Modal::Picker(_))));

        for c in "sport".chars() {
            update(&mut app, Msg::Key(key(c)));
        }
        let cmds = update(&mut app, Msg::Key(keycode(KeyCode::Enter)));
        assert!(matches!(cmds.as_slice(), [Cmd::SaveProfile]));
        assert!(app.modal.is_none());
        assert_eq!(app.profile.suggest("SOME NEW MERCHANT").unwrap().tag, "sport");
        assert!(app.profile.tags.iter().any(|t| t.name == "sport"));
    }

    #[test]
    fn tags_add_rename_and_delete_flow() {
        let mut app = test_app(vec![]);
        app.screen = Screen::Tags;

        update(&mut app, Msg::Key(key('a')));
        for c in "rent".chars() {
            update(&mut app, Msg::Key(key(c)));
        }
        update(&mut app, Msg::Key(keycode(KeyCode::Enter)));
        assert!(app.profile.tags.iter().any(|t| t.name == "rent"));

        update(&mut app, Msg::Key(key('r')));
        // Clear the pre-filled input, then type the new name.
        for _ in 0.."rent".len() {
            update(&mut app, Msg::Key(keycode(KeyCode::Backspace)));
        }
        for c in "housing".chars() {
            update(&mut app, Msg::Key(key(c)));
        }
        update(&mut app, Msg::Key(keycode(KeyCode::Enter)));
        assert!(app.profile.tags.iter().any(|t| t.name == "housing"));
        assert!(!app.profile.tags.iter().any(|t| t.name == "rent"));

        update(&mut app, Msg::Key(key('d')));
        assert!(matches!(app.modal, Some(Modal::Confirm(_))));
        update(&mut app, Msg::Key(key('y')));
        assert!(app.profile.tags.is_empty());
    }

    #[test]
    fn export_form_toggles_field_and_summary_flag() {
        let mut app = test_app(vec![entry("COOP LAUSANNE", -1.0)]);
        update(&mut app, Msg::Key(key('e')));
        let Some(Modal::ExportForm(form)) = &app.modal else { panic!("expected export form") };
        assert_eq!(form.field, ExportField::Path);
        assert!(form.summary);

        update(&mut app, Msg::Key(keycode(KeyCode::Tab)));
        let Some(Modal::ExportForm(form)) = &app.modal else { panic!("expected export form") };
        assert_eq!(form.field, ExportField::Summary);

        update(&mut app, Msg::Key(key(' ')));
        let Some(Modal::ExportForm(form)) = &app.modal else { panic!("expected export form") };
        assert!(!form.summary, "space toggles the checkbox off");

        let cmds = update(&mut app, Msg::Key(keycode(KeyCode::Enter)));
        match cmds.as_slice() {
            [Cmd::WriteExport(_, want_summary)] => assert!(!want_summary),
            other => panic!("expected a single WriteExport cmd, got {other:?}"),
        }
        assert!(app.modal.is_none());
    }
}
