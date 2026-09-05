use std::path::{Path, PathBuf};

type Fallible = Result<(), Box<dyn std::error::Error>>;

pub(super) fn run_split_sources(bundle: &str, bundle_path: Option<&Path>, out_dir: &str) -> Fallible {
    let dir = PathBuf::from(out_dir);
    let written = js_beautify_rs::sourcemap::reconstruct_bundle_tree(bundle, bundle_path, &dir)
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    eprintln!(
        "Split {} source file(s) into {}/reversed/",
        written.len(),
        dir.display()
    );
    for w in &written {
        println!("{w}");
    }
    Ok(())
}

pub(super) fn run_reconstruct_sourcemap(map_json: &str, out_dir: &str) -> Fallible {
    let dir = PathBuf::from(out_dir);
    let written = js_beautify_rs::sourcemap::reconstruct_tree(map_json, &dir)
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    eprintln!(
        "Reconstructed {} source file(s) into {}/reversed/",
        written.len(),
        dir.display()
    );
    for w in &written {
        println!("{w}");
    }
    Ok(())
}
