//! Keeps the lower layers independent of the upper ones.
//!
//! The dependency order is `model` <- `convert` <- `util`, `io` <- everything else. A file in a
//! lower layer may only name modules at or below its own layer, so pure data and algorithms can
//! be used and tested without the application, rendering, or UI. The upper layers (`app`,
//! `render`, `systems`, `gui`) still depend on each other in both directions; untangling that is
//! backlog item 2b.9 and 2b.7, and this test is extended when they are done.

use std::fs;
use std::path::{Path, PathBuf};

/// Top-level modules each lower layer may use, by directory under `src/`.
const RULES: &[(&str, &[&str])] = &[
    ("model", &["model"]),
    ("convert", &["model", "convert"]),
    ("util", &["model", "convert", "util"]),
    ("io", &["model", "convert", "util", "io"]),
];

/// Names `lib.rs` re-exports under another path.
fn canonical(module: &str) -> &str {
    match module {
        "nifti_loader" => "io",
        "components" => "app",
        other => other,
    }
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn test_lower_layers_do_not_import_upper_layers() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    for (layer, allowed) in RULES {
        let mut files = Vec::new();
        rust_files(&src.join(layer), &mut files);
        assert!(!files.is_empty(), "no sources found for layer {layer}");
        for file in files {
            for (number, line) in fs::read_to_string(&file).unwrap().lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                let mut rest = code;
                while let Some(at) = rest.find("crate::") {
                    rest = &rest[at + "crate::".len()..];
                    let module: String = rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    let module = canonical(&module);
                    if !allowed.contains(&module) {
                        violations.push(format!(
                            "{}:{}: layer `{layer}` imports `crate::{module}`",
                            file.strip_prefix(&src).unwrap().display(),
                            number + 1
                        ));
                    }
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "lower layers must not depend on upper ones:\n{}",
        violations.join("\n")
    );
}
