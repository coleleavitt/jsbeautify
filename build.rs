//! Embeds the git commit in the CLI version banner as `SemVer` build metadata:
//! `0.1.0+a1e1c69`, or `0.1.0+a1e1c69.dirty` when tracked files differ from
//! `HEAD`. Builds outside a git checkout keep the plain package version.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn main() {
    for path in ["build.rs", "Cargo.toml", "Cargo.lock", "src", "tests"] {
        println!("cargo:rerun-if-changed={path}");
    }

    let mut version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    if let Some(hash) = git(&["rev-parse", "--short", "HEAD"]) {
        let head_ref = git(&["symbolic-ref", "-q", "HEAD"]);
        for name in ["HEAD", "index", "packed-refs"].into_iter().chain(head_ref.as_deref()) {
            if let Some(path) = git(&["rev-parse", "--git-path", name])
                && std::path::Path::new(&path).exists()
            {
                println!("cargo:rerun-if-changed={path}");
            }
        }
        let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
        version = format!("{version}+{hash}{}", if dirty { ".dirty" } else { "" });
    }
    println!("cargo:rustc-env=JSB_VERSION={version}");
}
