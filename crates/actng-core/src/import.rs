use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::entry::Entry;
use crate::error::Error;

/// How to read a particular CSV layout: which columns hold what, and how
/// dates are formatted. Build one by hand for a known bank, or let
/// [`ImportProfile::detect`] infer it from the file itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportProfile {
    pub has_header: bool,
    pub date_column: Option<usize>,
    /// Columns joined (space-separated) into [`Entry::description`].
    pub description_columns: Vec<usize>,
    /// Set when the file has a single signed amount column.
    pub amount_column: Option<usize>,
    /// Set together with `credit_column` when debit/credit are split across
    /// two columns; the effective amount is `credit - debit`.
    pub debit_column: Option<usize>,
    pub credit_column: Option<usize>,
    /// chrono format strings, tried in order per row.
    pub date_formats: Vec<String>,
}

/// How the source bytes were decoded into text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    /// Fallback used when the bytes weren't valid UTF-8.
    Windows1252,
}

/// Result of an import: the profile that was used (detected or given), the
/// parsed entries, the file's layout fingerprint (see [`fingerprint`]), and
/// detection metadata useful for a debugging/reporting surface (`actng
/// scan`): the sniffed delimiter and encoding, and how many rows were
/// dropped as preamble or for lacking a usable description.
#[derive(Debug, Clone)]
pub struct Import {
    pub profile: ImportProfile,
    pub entries: Vec<Entry>,
    pub fingerprint: String,
    pub delimiter: u8,
    pub encoding: Encoding,
    pub skipped_rows: usize,
}

// Trailing `.?` tolerates the Hungarian `2019.04.19.` convention.
static RE_DATE_CELL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\d{1,4}[./-]\d{1,2}[./-]\d{1,4}\.?\s*$").unwrap());
// "27 Feb 2018", "10 Feb 18", "12-Dec-2019": day, month name, year.
static RE_DATE_CELL_MONTH_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*\d{1,2}[-\s][a-z]{3,9}[-\s]\d{2,4}\s*$").unwrap());

fn is_date_like(cell: &str) -> bool {
    RE_DATE_CELL.is_match(cell) || RE_DATE_CELL_MONTH_NAME.is_match(cell)
}

/// A row is assumed to be a header when none of its cells look like a date
/// or a number — real data rows almost always have at least one of those.
fn has_header(first_row: &[String]) -> bool {
    !first_row.iter().any(|cell| is_date_like(cell) || parse_amount(cell).is_some())
}

const DATE_HEADER_HINTS: &[&str] = &["date", "datum", "data", "valuta", "jour"];
const AMOUNT_HEADER_HINTS: &[&str] =
    &["montant", "amount", "betrag", "importo", "credit", "crédit", "debit", "débit"];
const DEBIT_HEADER_HINTS: &[&str] = &["debit", "débit", "debet", "outflow", "withdrawal", "kiadás"];
const CREDIT_HEADER_HINTS: &[&str] = &["credit", "crédit", "kredit", "inflow", "deposit", "bevétel"];
const DESCRIPTION_HEADER_HINTS: &[&str] = &[
    "texte", "text", "description", "libell", "buchung", "motivo", "communication", "notification",
    "détail", "detail", "beschreibung",
];
const DATE_FORMAT_CANDIDATES: &[&str] = &[
    "%Y-%m-%d", "%d.%m.%Y", "%m/%d/%Y", "%d/%m/%Y", "%Y/%m/%d", "%d-%m-%Y", "%d.%m.%y",
    "%m/%d/%y", "%d/%m/%y",
    // %d %b %y must precede %d %b %Y: chrono's %Y also accepts a 2-digit
    // year (as year 0018), so for an ambiguous 2-digit input the earlier
    // candidate wins the tie and gets the correct 2000s interpretation.
    "%d %b %y", "%d %b %Y", "%d-%b-%Y", "%Y.%m.%d.",
];

impl ImportProfile {
    /// Infer a profile from raw records (header row included, if any).
    ///
    /// Heuristics: a header row is assumed when the first row contains no
    /// date-like or numeric cell. Columns are then identified by header
    /// keywords (multilingual), falling back to content: the column whose
    /// values look like dates, the numeric column, and the longest remaining
    /// text column as description. The date format is chosen as the candidate
    /// that parses the most sample values, ties broken by how well the parsed
    /// sequence keeps a consistent sort order (which disambiguates D/M vs M/D).
    pub fn detect(records: &[Vec<String>]) -> Result<Self, Error> {
        let first = records.first().ok_or(Error::Empty)?;
        let has_header = has_header(first);
        let header: Vec<String> = if has_header {
            first.iter().map(|h| h.trim().to_lowercase()).collect()
        } else {
            Vec::new()
        };
        let data = if has_header { &records[1..] } else { records };
        if data.is_empty() {
            return Err(Error::Empty);
        }
        let samples: Vec<&Vec<String>> = data.iter().take(200).collect();
        let n_cols = samples.iter().map(|r| r.len()).max().unwrap_or(0);

        let content_share = |i: usize, pred: &dyn Fn(&str) -> bool| {
            let hits = samples.iter().filter(|r| pred(cell(r, i))).count();
            hits as f64 / samples.len() as f64
        };
        let header_match = |i: usize, hints: &[&str]| {
            header.get(i).is_some_and(|h| hints.iter().any(|hint| h.contains(hint)))
        };

        let date_column = (0..n_cols)
            .find(|&i| header_match(i, DATE_HEADER_HINTS))
            .or_else(|| (0..n_cols).find(|&i| content_share(i, &is_date_like) >= 0.6));

        let debit_column =
            (0..n_cols).filter(|i| Some(*i) != date_column).find(|&i| header_match(i, DEBIT_HEADER_HINTS));
        let credit_column = (0..n_cols)
            .filter(|i| Some(*i) != date_column && Some(*i) != debit_column)
            .find(|&i| header_match(i, CREDIT_HEADER_HINTS));

        let (amount_column, debit_column, credit_column) = if debit_column.is_some() && credit_column.is_some()
        {
            (None, debit_column, credit_column)
        } else {
            let amount_column = (0..n_cols).filter(|i| Some(*i) != date_column).find(|&i| {
                header_match(i, AMOUNT_HEADER_HINTS)
                    || content_share(i, &|c| !c.trim().is_empty() && parse_amount(c).is_some()) >= 0.6
            });
            (amount_column, None, None)
        };

        let taken = |i: &usize| {
            Some(*i) == date_column
                || Some(*i) == amount_column
                || Some(*i) == debit_column
                || Some(*i) == credit_column
        };
        let description_column = (0..n_cols)
            .filter(|i| !taken(i))
            .find(|&i| header_match(i, DESCRIPTION_HEADER_HINTS))
            .or_else(|| {
                (0..n_cols).filter(|i| !taken(i)).max_by_key(|&i| {
                    samples.iter().map(|r| cell(r, i).trim().chars().count()).sum::<usize>()
                })
            })
            .ok_or(Error::NoDescriptionColumn)?;

        let date_formats = match date_column {
            Some(col) => {
                let values: Vec<&str> = samples.iter().map(|r| cell(r, col)).collect();
                detect_date_formats(&values)
            }
            None => Vec::new(),
        };

        Ok(ImportProfile {
            has_header,
            date_column,
            description_columns: vec![description_column],
            amount_column,
            debit_column,
            credit_column,
            date_formats,
        })
    }

    /// Read a row's amount: either the single signed column, or `credit -
    /// debit` when the two are split. `None` if neither side has a value.
    fn read_amount(&self, row: &[String]) -> Option<f64> {
        if let Some(i) = self.amount_column {
            return parse_amount(cell(row, i));
        }
        let debit = self.debit_column.and_then(|i| parse_amount(cell(row, i)));
        let credit = self.credit_column.and_then(|i| parse_amount(cell(row, i)));
        match (debit, credit) {
            (None, None) => None,
            (debit, credit) => Some(credit.unwrap_or(0.0) - debit.unwrap_or(0.0).abs()),
        }
    }
}

/// Rank candidate date formats by how many of `values` they parse; ties are
/// broken by preferring the format under which the parsed dates best keep a
/// consistent (ascending or descending) order — statements are usually sorted,
/// which disambiguates M/D/Y from D/M/Y when all days are <= 12.
fn detect_date_formats(values: &[&str]) -> Vec<String> {
    let mut scored: Vec<(usize, usize, &str)> = DATE_FORMAT_CANDIDATES
        .iter()
        .map(|fmt| {
            let parsed: Vec<NaiveDate> = values
                .iter()
                .filter_map(|v| NaiveDate::parse_from_str(v.trim(), fmt).ok())
                .collect();
            let ascending =
                parsed.windows(2).filter(|w| w[0] > w[1]).count();
            let descending =
                parsed.windows(2).filter(|w| w[0] < w[1]).count();
            (parsed.len(), ascending.min(descending), *fmt)
        })
        .filter(|(count, _, _)| *count > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, fmt)| fmt.to_string()).collect()
}

const CURRENCY_SYMBOLS: [&str; 4] = ["CHF", "£", "€", "$"];

/// Parse an amount cell, tolerating Swiss/European formats (`1'234.56`,
/// `1 234,56`, `-12,30`) and a leading/trailing currency symbol (`£67.40`,
/// `CHF 12.30`). The symbol is only stripped when the remainder is fully
/// numeric, so a non-amount cell like `22FEB` is never mistaken for one.
/// Returns `None` for cells that aren't numbers.
pub(crate) fn parse_amount(cell: &str) -> Option<f64> {
    let mut s: String =
        cell.trim().chars().filter(|c| !matches!(c, '\'' | '\u{2019}' | ' ' | '\u{a0}')).collect();

    let negative = s.starts_with('-');
    if negative || s.starts_with('+') {
        s.remove(0);
    }
    for symbol in CURRENCY_SYMBOLS {
        if let Some(rest) = s.strip_prefix(symbol).or_else(|| s.strip_suffix(symbol)) {
            s = rest.to_string();
            break;
        }
    }

    if s.contains(',') && s.contains('.') {
        s = s.replace(',', "");
    } else {
        s = s.replace(',', ".");
    }
    if s.is_empty() || !s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let value: f64 = s.parse().ok()?;
    Some(if negative { -value } else { value })
}

fn parse_date(cell: &str, formats: &[String]) -> Option<NaiveDate> {
    let cell = cell.trim();
    formats.iter().find_map(|fmt| NaiveDate::parse_from_str(cell, fmt).ok())
}

fn cell(row: &[String], i: usize) -> &str {
    row.get(i).map(String::as_str).unwrap_or("")
}

/// Windows-1252 code points for bytes 0x80-0x9F (0xA0-0xFF map directly to
/// the same Unicode code point, as in Latin-1). Undefined bytes in this range
/// keep their C1-control code point, matching the WHATWG encoding standard.
const WINDOWS_1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

/// Decode bytes as Windows-1252, the fallback used when a file isn't valid
/// UTF-8 (common for exports from older banking systems).
fn decode_windows1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => WINDOWS_1252_HIGH[(b - 0x80) as usize],
            _ => b as char,
        })
        .collect()
}

const DELIMITER_CANDIDATES: [u8; 4] = [b',', b';', b'\t', b'|'];

/// Guess the field delimiter by trying each candidate against the first ~20
/// non-empty lines and scoring how consistently it splits each line into the
/// same (most common) field count; more fields and more consistency both
/// score higher. Defaults to `,` when no candidate does better than 1 field.
fn sniff_delimiter(text: &str) -> u8 {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).take(20).collect();
    let mut best = (b',', 0.0_f64);
    for &delim in &DELIMITER_CANDIDATES {
        let score = score_delimiter(&lines, delim);
        if score > best.1 {
            best = (delim, score);
        }
    }
    best.0
}

fn score_delimiter(lines: &[&str], delim: u8) -> f64 {
    if lines.is_empty() {
        return 0.0;
    }
    let counts: Vec<usize> = lines.iter().map(|l| l.split(delim as char).count()).collect();
    let m = mode(&counts);
    if m <= 1 {
        return 0.0;
    }
    let consistency = counts.iter().filter(|&&c| c == m).count() as f64 / counts.len() as f64;
    consistency * m as f64
}

/// Whether `text` looks like a delimited table (at least two columns, split
/// consistently across most lines) rather than free-form prose. Used by
/// `discover::is_bank_file` to decide whether a `.txt` file is worth
/// importing. Requires at least two non-empty lines — a single line can't
/// establish consistency, so it's never treated as delimited.
pub(crate) fn looks_delimited(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).take(20).collect();
    if lines.len() < 2 {
        return false;
    }
    DELIMITER_CANDIDATES.iter().any(|&delim| {
        let counts: Vec<usize> = lines.iter().map(|l| l.split(delim as char).count()).collect();
        let m = mode(&counts);
        if m < 2 {
            return false;
        }
        let consistency = counts.iter().filter(|&&c| c == m).count() as f64 / counts.len() as f64;
        consistency >= 0.8
    })
}

/// The most frequent value in `values`; ties favor the last one encountered.
fn mode(values: &[usize]) -> usize {
    values
        .iter()
        .copied()
        .max_by_key(|&v| values.iter().filter(|&&other| other == v).count())
        .unwrap_or(0)
}

/// Number of leading records whose field count differs from the modal field
/// count of the file. Metadata rows a bank export sometimes prepends before
/// the real header (account summaries, balances) usually have a different
/// column count and so get dropped before header/column detection runs.
fn skip_preamble(records: &[Vec<String>]) -> usize {
    let sample: Vec<usize> = records.iter().take(200).map(Vec::len).collect();
    let m = mode(&sample);
    records.iter().take_while(|r| r.len() != m).count()
}

/// Fingerprint a file's layout so a repeat export from the same source can
/// reuse a remembered [`ImportProfile`] (see `Profile::layouts`) instead of
/// re-running — or mis-running — auto-detection. Header-bearing files
/// fingerprint on the lowercased, joined header, which is stable across
/// exports and immune to incidental column-order matches; headerless files
/// fall back to delimiter + column count + a per-column content kind
/// (date-like / numeric / text).
fn fingerprint(records: &[Vec<String>], delimiter: u8) -> Option<String> {
    let first = records.first()?;
    if has_header(first) {
        return Some(first.iter().map(|h| h.trim().to_lowercase()).collect::<Vec<_>>().join("|"));
    }
    let samples: Vec<&Vec<String>> = records.iter().take(200).collect();
    let n_cols = samples.iter().map(|r| r.len()).max().unwrap_or(0);
    let kinds: String = (0..n_cols).map(|i| column_kind(&samples, i)).collect();
    Some(format!("{}:{}:{}", delimiter as char, n_cols, kinds))
}

/// `d` (date-like), `n` (numeric), or `t` (text/other), by majority content
/// across sampled rows — the same signal [`ImportProfile::detect`] uses to
/// pick out columns, reduced to one letter per column for fingerprinting.
fn column_kind(samples: &[&Vec<String>], i: usize) -> char {
    let share = |pred: &dyn Fn(&str) -> bool| {
        samples.iter().filter(|r| pred(cell(r, i))).count() as f64 / samples.len().max(1) as f64
    };
    if share(&is_date_like) >= 0.6 {
        'd'
    } else if share(&|c: &str| !c.trim().is_empty() && parse_amount(c).is_some()) >= 0.6 {
        'n'
    } else {
        't'
    }
}

/// Strip a UTF-8 BOM and decode as UTF-8, falling back to Windows-1252 when
/// the bytes aren't valid UTF-8.
pub(crate) fn decode_text(bytes: &[u8]) -> (String, Encoding) {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), Encoding::Utf8),
        Err(_) => (decode_windows1252(bytes), Encoding::Windows1252),
    }
}

/// Parsed records, the sniffed delimiter, the encoding the bytes were
/// decoded with, and how many leading preamble rows were dropped.
type ParsedRecords = (Vec<Vec<String>>, u8, Encoding, usize);

/// Decode, sniff the delimiter, parse into records, and drop the preamble —
/// every step of import that happens before a profile is picked.
fn parse_records<R: Read>(mut reader: R) -> Result<ParsedRecords, Error> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    let (text, encoding) = decode_text(&bytes);
    let delimiter = sniff_delimiter(&text);

    let mut csv_reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(text.as_bytes());
    let mut records: Vec<Vec<String>> = Vec::new();
    for record in csv_reader.records() {
        let fields: Vec<String> = record?.iter().map(str::to_string).collect();
        if fields.iter().all(|f| f.trim().is_empty()) {
            continue;
        }
        records.push(fields);
    }
    let preamble_dropped = skip_preamble(&records);
    records.drain(..preamble_dropped);
    Ok((records, delimiter, encoding, preamble_dropped))
}

fn build_import(
    records: Vec<Vec<String>>,
    delimiter: u8,
    encoding: Encoding,
    preamble_dropped: usize,
    profile: Option<ImportProfile>,
) -> Result<Import, Error> {
    let fingerprint = fingerprint(&records, delimiter).unwrap_or_default();
    let profile = match profile {
        Some(p) => p,
        None => ImportProfile::detect(&records)?,
    };
    let data = if profile.has_header && !records.is_empty() { &records[1..] } else { &records[..] };

    let mut skipped_rows = preamble_dropped;
    let entries = data
        .iter()
        .filter_map(|row| {
            let description = profile
                .description_columns
                .iter()
                .map(|&i| cell(row, i).trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if description.is_empty() {
                skipped_rows += 1;
                return None;
            }
            Some(Entry {
                date: profile
                    .date_column
                    .and_then(|i| parse_date(cell(row, i), &profile.date_formats)),
                description,
                amount: profile.read_amount(row),
                raw: row.clone(),
            })
        })
        .collect();

    Ok(Import { profile, entries, fingerprint, delimiter, encoding, skipped_rows })
}

/// Read entries from CSV bytes. With `profile: None` the layout is
/// auto-detected via [`ImportProfile::detect`]. A UTF-8 BOM is tolerated;
/// input that isn't valid UTF-8 is decoded as Windows-1252. The field
/// delimiter is sniffed among `,` `;` tab and `|`. Leading metadata rows
/// (see [`skip_preamble`]) are dropped before header/column detection.
/// Blank lines and rows with an empty description are skipped.
pub fn read_entries<R: Read>(reader: R, profile: Option<ImportProfile>) -> Result<Import, Error> {
    let (records, delimiter, encoding, preamble_dropped) = parse_records(reader)?;
    build_import(records, delimiter, encoding, preamble_dropped, profile)
}

/// Like [`read_entries`], but first checks the file's layout fingerprint
/// (see [`fingerprint`]) against `layouts` (typically `Profile::layouts`): a
/// hit reuses the stored profile instead of re-running auto-detection. The
/// returned `Import`'s `fingerprint` field is set either way, so on a miss
/// the caller can persist `import.profile` under `import.fingerprint`.
pub fn read_entries_with_layouts<R: Read>(
    reader: R,
    layouts: &HashMap<String, ImportProfile>,
) -> Result<Import, Error> {
    let (records, delimiter, encoding, preamble_dropped) = parse_records(reader)?;
    let profile = fingerprint(&records, delimiter).and_then(|fp| layouts.get(&fp).cloned());
    build_import(records, delimiter, encoding, preamble_dropped, profile)
}

/// Convenience wrapper over [`read_entries`] for a file on disk.
pub fn read_entries_from_path<P: AsRef<Path>>(
    path: P,
    profile: Option<ImportProfile>,
) -> Result<Import, Error> {
    read_entries(std::fs::File::open(path)?, profile)
}

/// Convenience wrapper over [`read_entries_with_layouts`] for a file on disk.
pub fn read_entries_from_path_with_layouts<P: AsRef<Path>>(
    path: P,
    layouts: &HashMap<String, ImportProfile>,
) -> Result<Import, Error> {
    read_entries_with_layouts(std::fs::File::open(path)?, layouts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_amount_variants() {
        assert_eq!(parse_amount("1'234.56"), Some(1234.56));
        assert_eq!(parse_amount("1 234,56"), Some(1234.56));
        assert_eq!(parse_amount("-12,30"), Some(-12.30));
        assert_eq!(parse_amount("1,234.56"), Some(1234.56));
        assert_eq!(parse_amount("COOP LAUSANNE"), None);
        assert_eq!(parse_amount(""), None);
    }

    #[test]
    fn fingerprint_matches_across_reexports_and_differs_across_layouts() {
        let csv_a = "Date;Payee;Amount\n01.02.2025;COOP LAUSANNE;-12,50\n";
        let csv_a_later_export = "Date;Payee;Amount\n01.03.2025;MIGROS RENENS;-8,90\n03.03.2025;COOP;-1,00\n";
        let csv_b = "Datum;Empfaenger;Betrag\n01.02.2025;COOP LAUSANNE;-12,50\n";

        let a = read_entries(csv_a.as_bytes(), None).unwrap();
        let a_later = read_entries(csv_a_later_export.as_bytes(), None).unwrap();
        let b = read_entries(csv_b.as_bytes(), None).unwrap();

        assert_eq!(a.fingerprint, a_later.fingerprint);
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn read_entries_with_layouts_reuses_a_stored_profile() {
        let csv = "01.02.2025,COOP LAUSANNE,-12.50\n";
        let fresh = read_entries(csv.as_bytes(), None).unwrap();

        let mut layouts = HashMap::new();
        layouts.insert(fresh.fingerprint.clone(), fresh.profile.clone());

        // A differently-shaped file that happens to share the stored
        // fingerprint reuses the remembered profile instead of re-detecting.
        let same_layout_different_content = "05.03.2025,MIGROS RENENS,-8.90\n";
        let reused = read_entries_with_layouts(same_layout_different_content.as_bytes(), &layouts).unwrap();
        assert_eq!(reused.profile, fresh.profile);
        assert_eq!(reused.entries.len(), 1);
        assert_eq!(reused.entries[0].amount, Some(-8.90));
    }

    #[test]
    fn parses_currency_symbols() {
        assert_eq!(parse_amount("£67.40"), Some(67.40));
        assert_eq!(parse_amount("+ £1,100.00"), Some(1100.0));
        assert_eq!(parse_amount("CHF 12.30"), Some(12.30));
        assert_eq!(parse_amount("-CHF12.30"), Some(-12.30));
        // A currency-looking suffix doesn't make an otherwise non-numeric
        // cell numeric.
        assert_eq!(parse_amount("22FEB"), None);
    }

    #[test]
    fn parses_month_name_dates() {
        assert_eq!(
            detect_date_formats(&["10 Feb 18"]).first().map(String::as_str),
            Some("%d %b %y")
        );
        assert_eq!(
            detect_date_formats(&["27 Feb 2018"]).first().map(String::as_str),
            Some("%d %b %Y")
        );
        assert_eq!(
            detect_date_formats(&["2019.04.19."]).first().map(String::as_str),
            Some("%Y.%m.%d.")
        );
    }

    #[test]
    fn decodes_windows_1252_fallback() {
        // "café" in Windows-1252: the trailing é is byte 0xE9, invalid as a
        // standalone UTF-8 continuation byte.
        let bytes = b"date,description,amount\n01.02.2025,caf\xe9,-1.00\n";
        let import = read_entries(&bytes[..], None).unwrap();
        assert_eq!(import.entries[0].description, "café");
    }

    #[test]
    fn sniffs_semicolon_delimiter() {
        let csv = "Date;Payee;Amount\n01.02.2025;COOP LAUSANNE;-12,50\n03.02.2025;MIGROS RENENS;-8,90\n";
        let import = read_entries(csv.as_bytes(), None).unwrap();
        assert_eq!(import.entries.len(), 2);
        assert_eq!(import.entries[0].amount, Some(-12.50));
        assert_eq!(import.delimiter, b';');
        assert_eq!(import.encoding, Encoding::Utf8);
    }

    #[test]
    fn reports_windows1252_encoding_and_semicolon_delimiter() {
        // "café" in Windows-1252: the trailing é is byte 0xE9, invalid UTF-8.
        let bytes = b"Date;Payee;Amount\n01.02.2025;caf\xe9;-12,50\n03.02.2025;MIGROS RENENS;-8,90\n";
        let import = read_entries(&bytes[..], None).unwrap();
        assert_eq!(import.delimiter, b';');
        assert_eq!(import.encoding, Encoding::Windows1252);
        assert_eq!(import.entries[0].description, "café");
    }

    #[test]
    fn skipped_rows_counts_preamble_and_empty_descriptions() {
        let csv = "Account details for:,ACME\nAvailable Balance,100.00\n\
                    Date,Description,Amount\n01.02.2025,COOP LAUSANNE,-12.50\n01.03.2025,,-3.00\n";
        let import = read_entries(csv.as_bytes(), None).unwrap();
        assert_eq!(import.entries.len(), 1);
        // 2 preamble rows dropped before the header, plus 1 data row with no
        // usable description.
        assert_eq!(import.skipped_rows, 3);
    }

    #[test]
    fn looks_delimited_accepts_tabular_txt_and_rejects_prose() {
        let tabular = "01.02.2025\tCOOP LAUSANNE\t-12.50\n03.02.2025\tMIGROS RENENS\t-8.90\n";
        assert!(looks_delimited(tabular));

        let prose = "Meeting notes:\nDiscussed budget, timeline, and resources.\nFollow up next week.\n";
        assert!(!looks_delimited(prose));

        // A single line can't establish consistency, even with a delimiter.
        assert!(!looks_delimited("just one, line, with, commas"));
    }

    #[test]
    fn sniffs_tab_delimiter() {
        let csv = "01.02.2025\tCOOP LAUSANNE\t-12,50\n03.02.2025\tMIGROS RENENS\t-8,90\n";
        let import = read_entries(csv.as_bytes(), None).unwrap();
        assert_eq!(import.entries.len(), 2);
        assert_eq!(import.entries[0].amount, Some(-12.50));
    }

    #[test]
    fn skips_metadata_preamble_before_header() {
        let csv = "Account details for:,ACME\nAvailable Balance,100.00\n\
                    Date,Description,Amount\n01.02.2025,COOP LAUSANNE,-12.50\n";
        let import = read_entries(csv.as_bytes(), None).unwrap();
        assert_eq!(import.entries.len(), 1);
        assert_eq!(import.entries[0].amount, Some(-12.50));
    }

    #[test]
    fn splits_debit_and_credit_columns() {
        let csv = "Date,Details,Debit,Credit\n\
                    01/09/2017,Random Name,,428.03\n\
                    02/09/2017,Random Bill,512.0,\n";
        let import = read_entries(csv.as_bytes(), None).unwrap();
        assert_eq!(import.entries[0].amount, Some(428.03));
        assert_eq!(import.entries[1].amount, Some(-512.0));
    }

    #[test]
    fn detects_columns_from_headers_and_content() {
        let records: Vec<Vec<String>> = vec![
            vec!["Date".into(), "Détails".into(), "Montant".into()],
            vec!["01.02.2025".into(), "COOP LAUSANNE".into(), "-12.50".into()],
            vec!["03.02.2025".into(), "MIGROS RENENS".into(), "-8.90".into()],
        ];
        let p = ImportProfile::detect(&records).unwrap();
        assert!(p.has_header);
        assert_eq!(p.date_column, Some(0));
        assert_eq!(p.description_columns, vec![1]);
        assert_eq!(p.amount_column, Some(2));
        assert_eq!(p.date_formats.first().map(String::as_str), Some("%d.%m.%Y"));
    }

    #[test]
    fn detects_headerless_files() {
        let records: Vec<Vec<String>> = vec![
            vec!["2025-02-01".into(), "COOP LAUSANNE".into()],
            vec!["2025-02-03".into(), "MIGROS RENENS".into()],
        ];
        let p = ImportProfile::detect(&records).unwrap();
        assert!(!p.has_header);
        assert_eq!(p.date_column, Some(0));
        assert_eq!(p.description_columns, vec![1]);
    }

    #[test]
    fn sort_order_disambiguates_day_month() {
        // All days <= 12: both %d/%m and %m/%d parse everything, but only
        // %d/%m keeps the file's descending order.
        let values = ["05/03/2025", "12/02/2025", "03/02/2025", "10/01/2025"];
        let formats = detect_date_formats(&values);
        assert_eq!(formats.first().map(String::as_str), Some("%d/%m/%Y"));
    }
}
