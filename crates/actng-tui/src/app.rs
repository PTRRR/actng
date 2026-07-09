use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::KeyEvent;

use actng_core::{Dataset, Encoding, Profile, Suggestion};

#[derive(Debug, Clone, Copy)]
pub enum Msg {
    Key(KeyEvent),
    /// Terminal size is always re-read from the `Frame` on the next draw, so
    /// there's nothing to carry here — the variant only exists to trigger a
    /// redraw promptly instead of waiting for the next tick.
    Resize,
    Tick,
}

/// Which screen the user is currently viewing. Switch with `1`-`5` or `Tab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Review,
    Entries,
    Tags,
    Files,
}

impl Screen {
    pub const ALL: [Screen; 5] = [Screen::Overview, Screen::Review, Screen::Entries, Screen::Tags, Screen::Files];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Overview => "Overview",
            Screen::Review => "Review",
            Screen::Entries => "Entries",
            Screen::Tags => "Tags",
            Screen::Files => "Files",
        }
    }
}

/// What a tag/category picker is being used for; decides which items are
/// listed and what confirming does.
#[derive(Debug, Clone)]
pub enum PickerContext {
    /// Confirm a tag for `entry_idx` (used by both Review and Entries retag).
    Tag { entry_idx: usize },
    /// Set `tag`'s category, picking from existing categories or a new one.
    Category { tag: String },
}

#[derive(Debug, Clone)]
pub struct Picker {
    pub context: PickerContext,
    pub query: String,
    pub selected: usize,
}

impl Picker {
    pub fn new(context: PickerContext) -> Self {
        Self { context, query: String::new(), selected: 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextPromptPurpose {
    AddTag,
    RenameTag,
}

#[derive(Debug, Clone)]
pub struct TextPrompt {
    pub purpose: TextPromptPurpose,
    pub input: String,
    /// For `RenameTag`, the tag being renamed.
    pub context: Option<String>,
}

#[derive(Debug, Clone)]
pub enum ConfirmContext {
    DeleteTag(String),
}

#[derive(Debug, Clone)]
pub struct Confirm {
    pub context: ConfirmContext,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportField {
    Path,
    Summary,
}

#[derive(Debug, Clone)]
pub struct ExportForm {
    pub path: String,
    pub summary: bool,
    pub field: ExportField,
}

#[derive(Debug, Clone)]
pub enum Modal {
    Picker(Picker),
    TextPrompt(TextPrompt),
    Confirm(Confirm),
    ExportForm(ExportForm),
    ExportSummary(Vec<(String, f64)>),
    Help,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub created: Instant,
}

impl Toast {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), created: Instant::now() }
    }
}

/// One reversible training step, enough to undo a Review/Entries confirmation.
#[derive(Debug, Clone)]
pub struct UndoRecord {
    pub entry_idx: usize,
    pub description: String,
    pub new_tag: String,
    /// The exact-match tag this confirmation overwrote, if the entry already
    /// had one (only possible in retag mode) and it differed from `new_tag`.
    pub previous_exact_tag: Option<String>,
}

/// Sort order for the Tags screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagSort {
    Name,
    Category,
    Trained,
}

impl TagSort {
    pub fn next(self) -> Self {
        match self {
            TagSort::Name => TagSort::Category,
            TagSort::Category => TagSort::Trained,
            TagSort::Trained => TagSort::Name,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TagSort::Name => "name",
            TagSort::Category => "category",
            TagSort::Trained => "trained",
        }
    }
}

/// Filter for the Entries screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryFilter {
    All,
    Tagged,
    Review,
    Tag(String),
}

#[derive(Debug, Clone)]
pub struct FileDetail {
    pub path: PathBuf,
    pub entries: usize,
    pub delimiter: u8,
    pub encoding: Encoding,
    pub skipped_rows: usize,
    /// Whether this file's layout was a remembered fingerprint hit, vs
    /// freshly auto-detected this run.
    pub layout_remembered: bool,
}

pub struct App {
    pub profile: Profile,
    pub profile_path: PathBuf,
    pub dir: PathBuf,
    pub dataset: Dataset,
    /// The per-file import results the current `dataset` was built from,
    /// kept around only for the Files screen's layout/encoding/delimiter
    /// detail — `Dataset` itself intentionally drops this once collected.
    /// Per-file detail for successfully imported files, in the same order as
    /// `dataset.sources` — kept alongside `Dataset` only for the Files
    /// screen, since `Dataset` itself intentionally drops per-file metadata
    /// once collected. Failed files are read from `dataset.failures`.
    pub file_details: Vec<FileDetail>,
    /// Parallel to `dataset.entries`.
    pub suggestions: Vec<Option<Suggestion>>,
    pub screen: Screen,
    pub modal: Option<Modal>,
    pub undo: Vec<UndoRecord>,
    pub toast: Option<Toast>,
    pub last_saved: Option<Instant>,
    pub should_quit: bool,

    // Review screen state.
    pub review_all: bool,
    pub review_cursor: usize,

    // Entries screen state.
    pub entries_cursor: usize,
    pub entries_filter: EntryFilter,
    pub entries_search: Option<String>,

    // Files screen state.
    pub files_cursor: usize,

    // Tags screen state.
    pub tags_cursor: usize,
    pub tags_sort: TagSort,
}

/// What the update loop wants the runtime to do; kept separate from `update`
/// so the loop itself stays pure and testable without touching disk.
#[derive(Debug, Clone)]
pub enum Cmd {
    SaveProfile,
    WriteExport(PathBuf, bool),
    Rescan,
    Quit,
}

impl App {
    pub fn new(
        profile: Profile,
        profile_path: PathBuf,
        dir: PathBuf,
        dataset: Dataset,
        file_details: Vec<FileDetail>,
    ) -> Self {
        let mut app = App {
            profile,
            profile_path,
            dir,
            suggestions: Vec::new(),
            dataset,
            file_details,
            screen: Screen::Overview,
            modal: None,
            undo: Vec::new(),
            toast: None,
            last_saved: None,
            should_quit: false,
            review_all: false,
            review_cursor: 0,
            entries_cursor: 0,
            entries_filter: EntryFilter::All,
            entries_search: None,
            files_cursor: 0,
            tags_cursor: 0,
            tags_sort: TagSort::Name,
        };
        app.recompute();
        app
    }

    /// Re-run `suggest` over every dataset entry. Called after any mutation
    /// to the profile's tagger (learn/unlearn/rename/remove), so the review
    /// queue and every screen stay perfectly consistent with no cache to
    /// invalidate.
    pub fn recompute(&mut self) {
        self.suggestions = self.dataset.entries.iter().map(|e| self.profile.suggest(&e.description)).collect();
        let queue_len = self.review_queue().len();
        if self.review_cursor >= queue_len {
            self.review_cursor = queue_len.saturating_sub(1);
        }
    }

    /// Indices into `dataset.entries` that currently need review (or, in
    /// retag mode, every entry).
    pub fn review_queue(&self) -> Vec<usize> {
        if self.review_all {
            (0..self.dataset.entries.len()).collect()
        } else {
            (0..self.dataset.entries.len()).filter(|&i| self.suggestions[i].is_none()).collect()
        }
    }

    pub fn tagged_count(&self) -> (usize, usize, usize) {
        let mut exact = 0;
        let mut bayes = 0;
        let mut review = 0;
        for s in &self.suggestions {
            match s {
                Some(sugg) if sugg.source == actng_core::Source::Exact => exact += 1,
                Some(_) => bayes += 1,
                None => review += 1,
            }
        }
        (exact, bayes, review)
    }

    pub fn set_toast(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast::new(message));
    }

    pub fn set_error(&mut self, err: impl std::fmt::Display) {
        self.modal = Some(Modal::Error(err.to_string()));
    }
}
