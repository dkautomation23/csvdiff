# Contributing

## Build and test

```bash
cargo build --release
cargo test
```

CI runs the same steps against a pinned toolchain and fails on any warning:

```bash
rustup toolchain install 1.98.0 --profile minimal --component clippy
cargo build --release --locked
cargo test --locked
cargo clippy --all-targets -- -D warnings
```

Run `cargo clippy --all-targets -- -D warnings` locally before opening a PR.
`Cargo.lock` is committed and CI builds with `--locked`; add dependencies
with `cargo add` rather than hand-editing the lockfile.

## Where tests live

There is no `tests/` directory — `src/compare.rs` (number/null normalization,
key joins, ignored columns, the three-pass diff) carries its own
`#[cfg(test)] mod tests` block. Put a new test next to the code it exercises.

## Adding a new normalization rule

A new "these should count as the same value" rule (a number format, a null
spelling) starts with a failing test in `src/compare.rs` that encodes the
exact input pair and the expected outcome — before the normalization code
changes. Add the counter-case too: a value that looks similar but must still
be reported as different (the README's `1000.00` vs. `1000.01` — money keeps
its cents — is the kind of edge this project cares about getting right).

If a change touches the comparison hot path, re-run the benchmark before and
after:

```bash
python bench/generate.py 1000000
cargo build --release
./target/release/csvdiff bench/before.csv bench/after.csv --key customer_id
```

## Commit style

Match the existing log (`git log --oneline`): `Area: what changed`, lower
case after the colon, imperative, no trailing period, no conventional-commit
prefixes. Examples from this repository:

```
Ship the benchmark that produces the README numbers, and run the tests in CI
README: state the Rust version CI actually proves
Release workflow: build a binary for Linux, macOS and Windows on a tag
```

## Pull requests

If a change affects what counts as "the same value" or the benchmark
numbers, update the README's comparison table or benchmark section in the
same PR. Small, focused PRs; say what file shapes you tested against.
