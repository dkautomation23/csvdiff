//! End-to-end checks that run the built binary directly - for behaviour that
//! lives at the CLI boundary (exit codes, files left on disk) and has no
//! single function worth unit testing in isolation.

use std::path::Path;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_csvdiff"))
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("write fixture file");
}

#[test]
fn an_existing_out_file_is_left_untouched_without_force() {
    let dir = std::env::temp_dir().join(format!("csvdiff-force-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let before = dir.join("before.csv");
    let after = dir.join("after.csv");
    let out = dir.join("out.csv");
    write(&before, "id,status\n1,active\n");
    write(&after, "id,status\n1,churned\n");
    write(&out, "sentinel - do not overwrite me\n");

    let status = bin()
        .arg(&before)
        .arg(&after)
        .args(["--key", "id", "--out"])
        .arg(&out)
        .status()
        .expect("run csvdiff");

    // The tool must refuse rather than silently clobber a file that was
    // already there - same exit code as its other "could not proceed" paths.
    assert_eq!(status.code(), Some(2), "must exit 2 when --out exists and --force was not given");

    let contents = std::fs::read_to_string(&out).expect("out file should still exist");
    assert_eq!(contents, "sentinel - do not overwrite me\n", "an existing --out file must not be overwritten without --force");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn force_overwrites_an_existing_out_file() {
    let dir = std::env::temp_dir().join(format!("csvdiff-force-test-yes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let before = dir.join("before.csv");
    let after = dir.join("after.csv");
    let out = dir.join("out.csv");
    write(&before, "id,status\n1,active\n");
    write(&after, "id,status\n1,churned\n");
    write(&out, "sentinel - do not overwrite me\n");

    let status = bin()
        .arg(&before)
        .arg(&after)
        .args(["--key", "id", "--out"])
        .arg(&out)
        .arg("--force")
        .status()
        .expect("run csvdiff");

    // Exit code 1 here means "differences were found and written", which is
    // exactly what should happen once --force allows the write to proceed.
    assert_eq!(status.code(), Some(1));

    let contents = std::fs::read_to_string(&out).expect("out file should still exist");
    assert!(contents.contains("changed"), "the diff should have been written over the sentinel: {contents:?}");

    std::fs::remove_dir_all(&dir).ok();
}
