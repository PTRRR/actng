use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// A single bank statement movement, as parsed from an imported file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Booking date, if a date column was found and the value parsed.
    pub date: Option<NaiveDate>,
    /// The free-text description this entry is tagged from.
    pub description: String,
    /// Amount, if the source file carries one.
    pub amount: Option<f64>,
    /// The raw CSV fields the entry was built from, for display and debugging.
    pub raw: Vec<String>,
}
