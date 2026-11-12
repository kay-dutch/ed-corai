//! Records which corai commit and which Coriolis code and data this build was made from.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (name, dir) in [
        ("CORIOLIS", "upstream/coriolis"),
        ("CORIOLIS_DATA", "upstream/coriolis-data"),
    ] {
        let d = root.join(dir);
        println!("cargo::rerun-if-changed={}", d.join(".git").display());
        if let Some(c) = git(&d, &["rev-parse", "HEAD"]) {
            println!("cargo::rustc-env=CORAI_{name}_COMMIT={c}");
        }
        if let Some(c) = git(&d, &["log", "-1", "--format=%cI"]) {
            println!("cargo::rustc-env=CORAI_{name}_DATE={c}");
        }
    }
    if let Some(c) = git(&root, &["rev-parse", "--short", "HEAD"]) {
        println!("cargo::rustc-env=CORAI_GIT_COMMIT={c}");
    }
}
