//! csvdiff - compare two CSV exports by key and report what actually changed.
//!
//!     csvdiff before.csv after.csv --key id
//!     csvdiff before.csv after.csv --key order_id,line_no --ignore updated_at
//!     csvdiff before.csv after.csv --key id --out changes.csv
//!
//! Three passes, because memory matters more than elegance on a ten-million-row
//! export:
//!
//! 1. read the old file, keep only `key hash -> row hash` (16 bytes a row);
//! 2. stream the new file: unknown key = added, same row hash = unchanged,
//!    otherwise remember the key as changed;
//! 3. read the old file again, but only for the keys that changed, and work out
//!    which columns moved.
//!
//! Nothing but the changed rows is ever held in full.

mod compare;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;

use compare::{changed_fields, hash_of, FieldChange, Normalisation};

#[derive(Parser, Debug)]
#[command(name = "csvdiff", about = "Compare two CSV exports by key")]
struct Args {
    /// The older export
    before: PathBuf,

    /// The newer export
    after: PathBuf,

    /// Key column(s), comma separated - the identity of a row
    #[arg(long)]
    key: String,

    /// Columns to ignore, comma separated (updated_at, exported_at, ...)
    #[arg(long, default_value = "")]
    ignore: String,

    /// Compare values literally: 1000 and 1000.00 become a change
    #[arg(long)]
    strict: bool,

    /// Also ignore case and whitespace inside text values
    #[arg(long)]
    loose_text: bool,

    /// Write every difference to this CSV
    #[arg(long)]
    out: Option<PathBuf>,

    /// How many examples to print per section
    #[arg(long, default_value_t = 5)]
    examples: usize,

    /// Field separator, if it is not a comma
    #[arg(long, default_value = ",")]
    delimiter: char,
}

struct Rows {
    columns: Vec<String>,
    key_indexes: Vec<usize>,
}

fn reader(path: &Path, delimiter: char) -> Result<csv::Reader<std::fs::File>, String> {
    csv::ReaderBuilder::new()
        .delimiter(delimiter as u8)
        .flexible(true)
        .from_path(path)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn header(path: &Path, delimiter: char, key: &[String]) -> Result<Rows, String> {
    let mut reader = reader(path, delimiter)?;
    let columns: Vec<String> = reader
        .headers()
        .map_err(|error| error.to_string())?
        .iter()
        .map(|value| value.trim().to_string())
        .collect();

    let mut key_indexes = Vec::new();
    for name in key {
        match columns.iter().position(|column| column.eq_ignore_ascii_case(name)) {
            Some(index) => key_indexes.push(index),
            None => {
                return Err(format!(
                    "{}: no column named '{name}'. Columns are: {}",
                    path.display(),
                    columns.join(", ")
                ))
            }
        }
    }
    Ok(Rows { columns, key_indexes })
}

fn key_of(record: &csv::StringRecord, indexes: &[usize]) -> String {
    indexes
        .iter()
        .map(|&index| record.get(index).unwrap_or("").trim())
        .collect::<Vec<_>>()
        .join("\u{1}")
}

fn row_values(record: &csv::StringRecord) -> Vec<String> {
    record.iter().map(|value| value.to_string()).collect()
}

/// Row hash over the comparable columns only, so ignored columns never show up
/// as a change in pass 2 either.
fn comparable(record: &csv::StringRecord, columns: &[String], ignore: &[String], rules: Normalisation) -> Vec<String> {
    columns
        .iter()
        .enumerate()
        .filter(|(_, column)| !ignore.iter().any(|ignored| ignored.eq_ignore_ascii_case(column)))
        .map(|(index, _)| compare::normalise(record.get(index).unwrap_or(""), rules))
        .collect()
}

#[derive(Default)]
struct Summary {
    added: Vec<String>,
    removed: Vec<String>,
    changed: Vec<(String, Vec<FieldChange>)>,
    duplicate_keys_before: usize,
    duplicate_keys_after: usize,
    malformed_before: Vec<(usize, usize)>,
    malformed_after: Vec<(usize, usize)>,
    rows_before: usize,
    rows_after: usize,
    column_changes: HashMap<String, usize>,
    value_moves: HashMap<String, HashMap<String, usize>>,
}

fn run(args: &Args) -> Result<Summary, String> {
    let key: Vec<String> = args.key.split(',').map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).collect();
    if key.is_empty() {
        return Err("--key needs at least one column".into());
    }
    let ignore: Vec<String> = args.ignore.split(',').map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).collect();

    let rules = if args.strict {
        Normalisation::default()
    } else {
        Normalisation { loose_text: args.loose_text, ..Normalisation::sensible() }
    };

    let before = header(&args.before, args.delimiter, &key)?;
    let after = header(&args.after, args.delimiter, &key)?;

    if before.columns != after.columns {
        let added: Vec<&String> = after.columns.iter().filter(|column| !before.columns.contains(column)).collect();
        let removed: Vec<&String> = before.columns.iter().filter(|column| !after.columns.contains(column)).collect();
        if !added.is_empty() || !removed.is_empty() {
            eprintln!(
                "note: the column sets differ - added: [{}], removed: [{}]. Comparison uses the new file's columns.",
                added.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", "),
                removed.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", "),
            );
        }
    }

    let mut summary = Summary::default();

    // ---- pass 1: the old file, as hashes ---------------------------------
    let mut old_hashes: HashMap<String, u64> = HashMap::new();
    let mut reader_before = reader(&args.before, args.delimiter)?;
    for (line, record) in reader_before.records().enumerate() {
        let record = record.map_err(|error| error.to_string())?;
        summary.rows_before += 1;
        if record.len() != before.columns.len() {
            // An unquoted separator inside a value shifts every column after
            // it. Comparing such a row produces nonsense differences.
            summary.malformed_before.push((line + 2, record.len()));
        }
        let key_value = key_of(&record, &before.key_indexes);
        let hash = hash_of(&comparable(&record, &before.columns, &ignore, rules));
        if old_hashes.insert(key_value, hash).is_some() {
            summary.duplicate_keys_before += 1;
        }
    }

    // ---- pass 2: the new file, streamed ----------------------------------
    let mut changed_keys: HashSet<String> = HashSet::new();
    let mut new_rows: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen_after: HashSet<String> = HashSet::new();

    let mut reader_after = reader(&args.after, args.delimiter)?;
    for (line, record) in reader_after.records().enumerate() {
        let record = record.map_err(|error| error.to_string())?;
        summary.rows_after += 1;
        if record.len() != after.columns.len() {
            summary.malformed_after.push((line + 2, record.len()));
        }
        let key_value = key_of(&record, &after.key_indexes);
        if !seen_after.insert(key_value.clone()) {
            summary.duplicate_keys_after += 1;
        }

        match old_hashes.remove(&key_value) {
            None => summary.added.push(key_value),
            Some(old_hash) => {
                let hash = hash_of(&comparable(&record, &after.columns, &ignore, rules));
                if hash != old_hash {
                    changed_keys.insert(key_value.clone());
                    new_rows.insert(key_value, row_values(&record));
                }
            }
        }
    }
    // Whatever is left was never seen in the new file.
    summary.removed = old_hashes.into_keys().collect();

    // ---- pass 3: only the rows that changed ------------------------------
    if !changed_keys.is_empty() {
        let mut reader_before = reader(&args.before, args.delimiter)?;
        for record in reader_before.records() {
            let record = record.map_err(|error| error.to_string())?;
            let key_value = key_of(&record, &before.key_indexes);
            if !changed_keys.contains(&key_value) {
                continue;
            }
            let Some(new_row) = new_rows.get(&key_value) else { continue };
            let changes = changed_fields(&after.columns, &row_values(&record), new_row, &ignore, rules);
            for change in &changes {
                *summary.column_changes.entry(change.column.clone()).or_default() += 1;
                *summary
                    .value_moves
                    .entry(change.column.clone())
                    .or_default()
                    .entry(format!("{} -> {}", short(&change.before), short(&change.after)))
                    .or_default() += 1;
            }
            if !changes.is_empty() {
                summary.changed.push((key_value, changes));
            }
        }
    }

    summary.added.sort();
    summary.removed.sort();
    summary.changed.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(summary)
}

fn short(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "(empty)".into();
    }
    if value.chars().count() > 24 {
        format!("{}...", value.chars().take(21).collect::<String>())
    } else {
        value.to_string()
    }
}

fn display_key(key: &str) -> String {
    key.replace('\u{1}', " | ")
}

fn print_summary(summary: &Summary, args: &Args, elapsed: f64) {
    let total = summary.added.len() + summary.removed.len() + summary.changed.len();
    println!(
        "\n{} row(s) before, {} after - {} added, {} removed, {} changed  ({:.1}s)",
        summary.rows_before,
        summary.rows_after,
        summary.added.len(),
        summary.removed.len(),
        summary.changed.len(),
        elapsed,
    );

    if !summary.malformed_before.is_empty() || !summary.malformed_after.is_empty() {
        let expected_before = summary.malformed_before.len();
        let expected_after = summary.malformed_after.len();
        println!(
            "
WARNING: {} row(s) in the old file and {} in the new one have the wrong number of 
         fields - usually an unquoted separator inside a value. Every column after it 
         is shifted, so their differences below are not real.",
            expected_before, expected_after
        );
        for (line, fields) in summary.malformed_before.iter().chain(summary.malformed_after.iter()).take(3) {
            println!("         line {line}: {fields} field(s)");
        }
    }

    if summary.duplicate_keys_before > 0 || summary.duplicate_keys_after > 0 {
        // A duplicated key means the comparison itself is unreliable, so it is
        // said out loud rather than buried.
        println!(
            "\nWARNING: duplicate keys - {} in the old file, {} in the new one. \
             Only the last row of each key was compared; fix the key or the export.",
            summary.duplicate_keys_before, summary.duplicate_keys_after
        );
    }

    if total == 0 {
        println!("\nThe two files match on every compared column.\n");
        return;
    }

    if !summary.column_changes.is_empty() {
        println!("\nWhat moved, by column");
        let mut columns: Vec<(&String, &usize)> = summary.column_changes.iter().collect();
        columns.sort_by(|a, b| b.1.cmp(a.1));
        for (column, count) in columns {
            println!("  {column:<24} {count} row(s)");
            if let Some(moves) = summary.value_moves.get(column) {
                let mut top: Vec<(&String, &usize)> = moves.iter().collect();
                top.sort_by(|a, b| b.1.cmp(a.1));
                for (transition, times) in top.into_iter().take(3) {
                    println!("      {times:>6} x  {transition}");
                }
                if moves.len() > 3 {
                    println!("      {:>6}    other transitions", moves.len() - 3);
                }
            }
        }
    }

    let sections: [(&str, Vec<String>); 3] = [
        ("Added", summary.added.iter().map(|key| display_key(key)).collect()),
        ("Removed", summary.removed.iter().map(|key| display_key(key)).collect()),
        (
            "Changed",
            summary
                .changed
                .iter()
                .map(|(key, changes)| {
                    let detail = changes
                        .iter()
                        .take(3)
                        .map(|change| format!("{}: {} -> {}", change.column, short(&change.before), short(&change.after)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{}  ({detail})", display_key(key))
                })
                .collect(),
        ),
    ];

    for (title, items) in sections {
        if items.is_empty() {
            continue;
        }
        println!("\n{title} ({})", items.len());
        for item in items.iter().take(args.examples) {
            println!("  {item}");
        }
        if items.len() > args.examples {
            println!("  ... and {} more", items.len() - args.examples);
        }
    }
    println!();
}

fn write_out(path: &Path, summary: &Summary) -> Result<(), String> {
    let mut writer = csv::Writer::from_path(path).map_err(|error| error.to_string())?;
    writer
        .write_record(["change", "key", "column", "before", "after"])
        .map_err(|error| error.to_string())?;

    for key in &summary.added {
        writer.write_record(["added", &display_key(key), "", "", ""]).map_err(|e| e.to_string())?;
    }
    for key in &summary.removed {
        writer.write_record(["removed", &display_key(key), "", "", ""]).map_err(|e| e.to_string())?;
    }
    for (key, changes) in &summary.changed {
        for change in changes {
            writer
                .write_record(["changed", &display_key(key), &change.column, &change.before, &change.after])
                .map_err(|e| e.to_string())?;
        }
    }
    writer.flush().map_err(|error| error.to_string())?;
    println!("differences -> {}", path.display());
    Ok(())
}

fn main() {
    let args = Args::parse();
    let started = Instant::now();

    let summary = match run(&args) {
        Ok(summary) => summary,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    print_summary(&summary, &args, started.elapsed().as_secs_f64());

    if let Some(path) = &args.out {
        if let Err(error) = write_out(path, &summary) {
            eprintln!("could not write {}: {error}", path.display());
            std::process::exit(2);
        }
    }

    let differences = summary.added.len() + summary.removed.len() + summary.changed.len();
    std::process::exit(if differences > 0 { 1 } else { 0 });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_keys_are_joined_unambiguously() {
        let record = csv::StringRecord::from(vec!["A|B", "C", "x"]);
        let single = key_of(&record, &[0]);
        let composite = key_of(&record, &[0, 1]);
        // A literal "|" inside a value must not look like a key separator.
        assert_ne!(single, composite);
        assert_eq!(display_key(&composite), "A|B | C");
    }

    #[test]
    fn long_values_are_shortened_for_the_summary() {
        assert_eq!(short(""), "(empty)");
        assert_eq!(short("short"), "short");
        assert_eq!(short(&"x".repeat(40)).len(), 24);
    }
}
