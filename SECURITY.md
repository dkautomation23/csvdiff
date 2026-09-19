# Security Policy

csvdiff reads two CSV files you give it — often exports containing customer
or business data — and can write a third CSV listing every difference. The
security-relevant behavior is bounded by that: parsing untrusted CSV input,
and generating CSV output that might later be opened in a spreadsheet.

## What counts as a vulnerability here

- **CSV/formula injection.** A cell value that, once written into `--out`
  output, opens as a spreadsheet formula (a cell starting with `=`, `+`,
  `-`, or `@`) instead of plain text when the result is opened in Excel or
  Sheets.
- A crafted CSV (a pathological quoting pattern, an extreme field or line
  length, a huge number of columns) that causes a crash beyond a clean
  handled error, memory use wildly out of proportion to the file size, or
  reads/writes outside the two files you named and the `--out` path you
  gave.
- The row-streaming design being defeated — csvdiff is built to hold only
  changed rows in memory (see "How it stays small" in the README); a crafted
  file that forces it to buffer the entire dataset instead, to the point of
  exhausting memory on a file that should fit comfortably, is a vulnerability
  against that design guarantee.

Report these.

## What is not a vulnerability

- Values that normalize to "the same" being treated as unchanged (`1000` and
  `1000.00`, or the documented null spellings `NULL`/`None`/`NaN`/`\N`) —
  that's the documented normalization; `--strict` turns it off.
- A duplicate key comparing against "the last row it saw" rather than every
  occurrence — documented limitation.
- Dates compared as text rather than parsed and normalized — documented; a
  format guess per column was a deliberate non-goal.
- A malformed row (wrong field count) being flagged as unreliable instead of
  diffed — that's the intended behavior, not a bug.

## Reporting a vulnerability

Preferred: open a report through
[GitHub Private vulnerability reporting](https://github.com/dkautomation23/csvdiff/security/advisories/new)
on this repository.

Alternative: email **hello@dkautomation.dev** with `csvdiff` in the subject
line. If reproducing the issue needs a file with real customer data, don't
attach it — describe the column shapes and value patterns instead, or
reproduce with a synthetic file (`bench/generate.py` builds one from a fixed
seed).

Please include:
- the csvdiff version (`csvdiff --version`) and OS,
- the exact command and flags you ran,
- a minimal reproducing CSV (synthetic, not a real export).

**First response within 3 business days.** After triage we'll tell you the
expected timeline for a fix and credit you in the release notes, if you want
that.

## Supported versions

Only the latest release is supported. If you're on an older tag, please
upgrade before reporting — the issue may already be fixed.
