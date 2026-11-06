//! Keep the same generated-artifact checks in local workspace tests and CI.
//! Builds still consume the committed artifacts without Node or Python.
use std::{path::Path, process::Command};

fn check(root: &Path, program: &str, args: &[&str], preparation: &str) {
    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .unwrap_or_else(|error| panic!("cannot run {program}: {error}; {preparation}"));
    assert!(status.success(), "{program} {args:?} failed; {preparation}");
}

#[test]
fn committed_generated_artifacts_are_current() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // generate.mjs checks scene docs/defaults, JSON Schema and SDK types by
    // running the Rust generator; generate-std.mjs checks runtime JS and help.
    for script in ["generate.mjs", "generate-std.mjs"] {
        check(
            &root,
            "node",
            &[&format!("packages/ts/{script}"), "--check"],
            "install Node and run npm ci --prefix packages/ts; regenerate with scripts/generate-docs and node packages/ts/generate-std.mjs",
        );
    }
    check(
        &root,
        if cfg!(windows) { "python" } else { "python3" },
        &["scripts/generate-notice", "--check"],
        "install Python 3, run cargo fetch --locked, then scripts/generate-notice",
    );
}
