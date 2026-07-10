//! Core library for parsing and tagging bank statement entries.
//!
//! The pipeline has three stages:
//!
//! 1. **Import** ([`read_entries`], [`ImportProfile`]) — load a CSV of unknown
//!    layout. Column roles (date, description, amount) and the date format are
//!    auto-detected from headers and cell contents, or can be given explicitly.
//! 2. **Normalize** ([`normalize`]) — strip noise from the free-text
//!    description (dates, card masks, amounts, reference numbers), fold accents
//!    and case, and split into tokens. IBANs and phone numbers are extracted as
//!    stable counterparty identifiers.
//! 3. **Tag** ([`Tagger`]) — an exact-match memory keyed on the normalized
//!    token set answers first (deterministic, learned from user decisions);
//!    otherwise a Naive Bayes classifier over tokens suggests a tag with a
//!    confidence. Low-confidence entries get no suggestion and should go to a
//!    review queue, whose answers feed back in via [`Tagger::learn`].
//!
//! ```no_run
//! use actng_core::{read_entries_from_path, Tagger};
//!
//! let import = read_entries_from_path("statement.csv", None)?;
//! let mut tagger = Tagger::default();
//! tagger.learn("ACHAT TWINT DU 06.11.2025 GRIMPER.CH LAUSANNE (CH)", "climbing");
//!
//! for entry in &import.entries {
//!     match tagger.suggest(&entry.description) {
//!         Some(s) => println!("{} -> {} ({:.0}%, {:?})", entry.description, s.tag, s.confidence * 100.0, s.source),
//!         None => println!("{} -> needs review", entry.description),
//!     }
//! }
//! # Ok::<(), actng_core::Error>(())
//! ```

pub mod bayes;
pub mod discover;
pub mod entry;
pub mod error;
pub mod export;
pub mod import;
pub mod normalize;
pub mod profile;
pub mod tagger;

pub use bayes::NaiveBayes;
pub use discover::{collect, discover, import_dir, Dataset, FileImport};
pub use entry::Entry;
pub use error::Error;
pub use export::{write_csv, Summary};
pub use import::{
    read_entries, read_entries_from_path, read_entries_from_path_with_layouts,
    read_entries_with_layouts, Encoding, Import, ImportProfile,
};
pub use normalize::{normalize, Normalized};
pub use profile::{Override, Profile, Tag, CURRENT_VERSION};
pub use tagger::{Source, Suggestion, TagStats, Tagger};
