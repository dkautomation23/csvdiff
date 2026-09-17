#!/usr/bin/env python3
"""Build the pair of CSV exports the README timings are measured on.

Stdlib only, so the benchmark can be reproduced on a clean machine:

    python bench/generate.py 1000000
    cargo build --release
    ./target/release/csvdiff bench/before.csv bench/after.csv --key customer_id
    ./target/release/csvdiff bench/before.csv bench/after.csv --key customer_id --ignore updated_at

The shape is the one that makes a real migration report unreadable: every row
carries a timestamp that moves on every export, so without --ignore almost the
whole file looks changed, while the changes that matter are a fraction of a
percent underneath.
"""
from __future__ import annotations

import csv
import random
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
COLUMNS = ["customer_id", "email", "status", "plan", "mrr", "country", "updated_at"]
STATUSES = ["active", "trial", "churned", "paused"]
PLANS = ["starter", "growth", "scale"]
COUNTRIES = ["DE", "GB", "US", "FR", "BG", "PL"]

# Fixed so two runs on two machines compare the same work.
SEED = 20260822
REMOVED = 5_000          # rows present before and gone after
ADDED = 5_000            # rows that only exist after
CHANGED_RATE = 0.0133    # share of surviving rows with a real status change


def row(rng: random.Random, customer_id: int, stamp: str) -> list:
    return [
        customer_id,
        f"user{customer_id}@example.com",
        rng.choice(STATUSES),
        rng.choice(PLANS),
        f"{rng.randrange(19, 499)}.00",
        rng.choice(COUNTRIES),
        stamp,
    ]


def main() -> int:
    rows = int(sys.argv[1]) if len(sys.argv) > 1 else 1_000_000
    if rows <= REMOVED + ADDED:
        print(f"need more than {REMOVED + ADDED} rows")
        return 1

    rng = random.Random(SEED)
    before = [row(rng, i, "2026-07-01") for i in range(1, rows + 1)]

    after = []
    for record in before[: rows - REMOVED]:          # the tail is the removed block
        copy = list(record)
        copy[-1] = "2026-08-22"                      # the column that always moves
        if rng.random() < CHANGED_RATE:
            copy[2] = "churned"
        after.append(copy)
    for i in range(ADDED):
        after.append(row(rng, rows + 1 + i, "2026-08-22"))

    for name, data in (("before.csv", before), ("after.csv", after)):
        path = HERE / name
        with path.open("w", newline="", encoding="utf-8") as handle:
            writer = csv.writer(handle)
            writer.writerow(COLUMNS)
            writer.writerows(data)
        print(f"{path.name}: {len(data)} rows, {path.stat().st_size / 1e6:.0f} MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
