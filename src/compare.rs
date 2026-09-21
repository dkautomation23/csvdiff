//! Comparing values and rows.
//!
//! The hard part of diffing exports is not the diff, it is deciding what counts
//! as a change. Two systems write the same number as `1000`, `1000.00` and
//! `1,000.00`; the same date as `2026-08-22` and `22/08/2026`; the same empty
//! cell as ``, ` `, `NULL` and `\N`. Reporting those as differences is how a
//! migration report ends up with 40 000 "changes" nobody reads.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, Default)]
pub struct Normalisation {
    /// Treat 1000, 1000.00 and 1,000.00 as the same value.
    pub numbers: bool,
    /// Treat empty, whitespace, NULL, NaN and \N as the same value.
    pub nulls: bool,
    /// Ignore case and surrounding whitespace in text.
    pub loose_text: bool,
}

impl Normalisation {
    /// What most people mean by "did this row change?".
    pub fn sensible() -> Self {
        Self { numbers: true, nulls: true, loose_text: false }
    }
}

const NULLS: [&str; 6] = ["", "null", "nil", "none", "nan", "\\n"];

/// Reduce a cell to the form used for comparison.
pub fn normalise(value: &str, rules: Normalisation) -> String {
    let trimmed = value.trim();

    if rules.nulls && NULLS.contains(&trimmed.to_ascii_lowercase().as_str()) {
        return String::new();
    }

    if rules.numbers {
        if let Some(number) = as_number(trimmed) {
            // Format through f64 so 1000, 1000.0 and 1,000.00 converge, but
            // keep enough digits that money does not get rounded away.
            return format!("{number:.6}").trim_end_matches('0').trim_end_matches('.').to_string();
        }
    }

    if rules.loose_text {
        return trimmed.to_lowercase();
    }
    trimmed.to_string()
}

/// Parse a number written in either convention, or not at all.
fn as_number(value: &str) -> Option<f64> {
    if value.is_empty() || !value.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    if value.chars().any(|c| c.is_alphabetic()) {
        return None;                     // "12 units" is text, not a quantity
    }

    let cleaned = value.replace(' ', "");
    let cleaned = match (cleaned.rfind(','), cleaned.rfind('.')) {
        // Both separators: the rightmost one is the decimal point.
        (Some(comma), Some(dot)) if comma > dot => cleaned.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => cleaned.replace(',', ""),
        // Only commas: decimal separator if it is followed by exactly 2 digits,
        // otherwise a thousands separator.
        (Some(comma), None) => {
            if cleaned.len() - comma - 1 == 2 && cleaned.matches(',').count() == 1 {
                cleaned.replace(',', ".")
            } else {
                cleaned.replace(',', "")
            }
        }
        _ => cleaned,
    };
    cleaned.parse::<f64>().ok()
}

pub fn hash_of(values: &[String]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for value in values {
        value.hash(&mut hasher);
        // Separator so ["ab","c"] and ["a","bc"] do not collide.
        0xffu8.hash(&mut hasher);
    }
    hasher.finish()
}

/// One field that differs between the two versions of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChange {
    pub column: String,
    pub before: String,
    pub after: String,
}

/// Compare two rows column by column, ignoring the columns asked for.
pub fn changed_fields(
    columns: &[String],
    before: &[String],
    after: &[String],
    ignore: &[String],
    rules: Normalisation,
) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        if ignore.iter().any(|ignored| ignored.eq_ignore_ascii_case(column)) {
            continue;
        }
        let old = before.get(index).map(String::as_str).unwrap_or("");
        let new = after.get(index).map(String::as_str).unwrap_or("");
        if normalise(old, rules) != normalise(new, rules) {
            changes.push(FieldChange {
                column: column.clone(),
                before: old.to_string(),
                after: new.to_string(),
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn the_same_number_written_three_ways_is_one_value() {
        let rules = Normalisation::sensible();
        let expected = normalise("1000", rules);
        assert_eq!(normalise("1000.00", rules), expected);
        assert_eq!(normalise("1,000.00", rules), expected);
        assert_eq!(normalise(" 1000 ", rules), expected);
    }

    #[test]
    fn european_decimals_are_understood() {
        let rules = Normalisation::sensible();
        assert_eq!(normalise("1.234,56", rules), normalise("1234.56", rules));
        assert_eq!(normalise("0,50", rules), normalise("0.5", rules));
    }

    #[test]
    fn money_keeps_its_cents() {
        let rules = Normalisation::sensible();
        assert_ne!(normalise("1000.00", rules), normalise("1000.01", rules));
        assert_ne!(normalise("0.1", rules), normalise("0.2", rules));
    }

    #[test]
    fn text_that_contains_digits_is_not_a_number() {
        let rules = Normalisation::sensible();
        assert_eq!(normalise("12 units", rules), "12 units");
        assert_ne!(normalise("2026-08-22", rules), normalise("20260822", rules));
    }

    #[test]
    fn the_many_spellings_of_empty_collapse() {
        let rules = Normalisation::sensible();
        for value in ["", "  ", "NULL", "null", "None", "NaN", "\\N"] {
            assert_eq!(normalise(value, rules), "", "{value:?} should be empty");
        }
    }

    #[test]
    fn nulls_stay_distinct_when_normalisation_is_off() {
        let strict = Normalisation::default();
        assert_ne!(normalise("NULL", strict), normalise("", strict));
        assert_ne!(normalise("1000.00", strict), normalise("1000", strict));
    }

    #[test]
    fn loose_text_only_applies_when_asked_for() {
        let sensible = Normalisation::sensible();
        let loose = Normalisation { loose_text: true, ..Normalisation::sensible() };
        assert_ne!(normalise("Acme GmbH", sensible), normalise("acme gmbh", sensible));
        assert_eq!(normalise("Acme GmbH", loose), normalise("acme gmbh", loose));
    }

    #[test]
    fn field_comparison_names_what_moved() {
        let columns = strings(&["id", "status", "amount"]);
        let changes = changed_fields(
            &columns,
            &strings(&["1", "active", "1000"]),
            &strings(&["1", "churned", "1000.00"]),
            &[],
            Normalisation::sensible(),
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].column, "status");
        assert_eq!(changes[0].before, "active");
        assert_eq!(changes[0].after, "churned");
    }

    #[test]
    fn ignored_columns_do_not_produce_changes() {
        let columns = strings(&["id", "updated_at", "status"]);
        let changes = changed_fields(
            &columns,
            &strings(&["1", "2026-08-01", "active"]),
            &strings(&["1", "2026-08-22", "active"]),
            &strings(&["UPDATED_AT"]),      // matching is case-insensitive
            Normalisation::sensible(),
        );
        assert!(changes.is_empty());
    }

    #[test]
    fn a_short_row_is_compared_as_if_the_missing_cells_were_empty() {
        let columns = strings(&["id", "email", "phone"]);
        let changes = changed_fields(
            &columns,
            &strings(&["1", "a@example.com"]),
            &strings(&["1", "a@example.com", "+49170"]),
            &[],
            Normalisation::sensible(),
        );
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].column, "phone");
    }

    #[test]
    fn row_hashes_do_not_collide_on_field_boundaries() {
        assert_ne!(hash_of(&strings(&["ab", "c"])), hash_of(&strings(&["a", "bc"])));
        assert_eq!(hash_of(&strings(&["a", "b"])), hash_of(&strings(&["a", "b"])));
    }
}
