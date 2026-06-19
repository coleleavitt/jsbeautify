use js_beautify_rs::cross_version::{AlignConfig, CrossVersionAligner};
use js_beautify_rs::{Options, beautify};
use std::fs;
use std::path::PathBuf;

pub(super) fn run_cross_version_align(
    sourcemap_path: Option<&PathBuf>,
    names_json_path: Option<&PathBuf>,
    align_with_path: Option<&PathBuf>,
    align_output_path: Option<&PathBuf>,
    output_path: Option<&String>,
    source_code: &str,
    options: &Options,
    raw: bool,
    bun_extract: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = AlignConfig {
        sourcemap_path: sourcemap_path.map(|p| p.to_string_lossy().to_string()),
        align_with: align_with_path.map(|p| p.to_string_lossy().to_string()),
        hash_depth: 12,
        use_slot_naming: false,
    };

    let mut aligner = CrossVersionAligner::new(config);

    if let Some(names_path) = names_json_path {
        eprintln!("[ALIGN] Loading names.json: {}", names_path.display());
        let names_json = fs::read_to_string(names_path)?;
        let stable_count = aligner.load_names_json(&names_json)?;
        eprintln!("[ALIGN] Loaded {stable_count} stable name mappings from names.json");
    } else if let Some(smap_path) = sourcemap_path {
        eprintln!("[ALIGN] Loading sourcemap: {}", smap_path.display());
        let sourcemap_json = fs::read_to_string(smap_path)?;
        let stable_count = aligner.load_sourcemap(&sourcemap_json, source_code)?;
        eprintln!("[ALIGN] Loaded {stable_count} stable name mappings from sourcemap");
    }

    if bun_extract {
        eprintln!("[ALIGN] Extracting names from Bun bundle patterns...");
        let bun_count = aligner.load_bun_names(source_code);
        eprintln!("[ALIGN] Extracted {bun_count} name mappings from MR/this.name/displayName");
    }

    if let Some(target_path) = align_with_path {
        eprintln!("[ALIGN] Loading target bundle: {}", target_path.display());
        let target_code = fs::read_to_string(target_path)?;

        eprintln!("[ALIGN] Aligning statements on RAW code...");
        let (aligned_source, aligned_target, stats) = aligner.align_sources(source_code, &target_code);

        eprintln!("[ALIGN] === Results ===");
        eprintln!(
            "[ALIGN] Matched: {} / {} ({:.1}%)",
            stats.matched_statements,
            stats.source_statements,
            stats.match_rate()
        );
        eprintln!("[ALIGN] Source replacements: {}", stats.source_replacements);
        eprintln!("[ALIGN] Target replacements: {}", stats.target_replacements);
        eprintln!("[ALIGN] Canonical names generated: {}", stats.canonical_names_generated);

        let (source_out, target_out) = if raw {
            (aligned_source, aligned_target)
        } else {
            eprintln!("[ALIGN] Beautifying aligned source...");
            let source_beautified = beautify(&aligned_source, options)?;
            eprintln!("[ALIGN] Source beautified: {} lines", source_beautified.lines().count());

            eprintln!("[ALIGN] Beautifying aligned target...");
            let target_beautified = beautify(&aligned_target, options)?;
            eprintln!("[ALIGN] Target beautified: {} lines", target_beautified.lines().count());
            (source_beautified, target_beautified)
        };

        if let Some(align_out) = align_output_path {
            fs::write(align_out, &target_out)?;
            eprintln!("[ALIGN] Target output written to {}", align_out.display());
        }

        if let Some(out_path) = output_path {
            fs::write(out_path, &source_out)?;
            eprintln!("[ALIGN] Source output written to {out_path}");
        } else {
            println!("{source_out}");
        }
    } else {
        eprintln!("[ALIGN] Beautifying source...");
        let source_beautified = beautify(source_code, options)?;
        eprintln!("[ALIGN] Source beautified: {} lines", source_beautified.lines().count());

        if let Some(out_path) = output_path {
            fs::write(out_path, &source_beautified)?;
            eprintln!("[ALIGN] Source output written to {out_path}");
        } else {
            println!("{source_beautified}");
        }
    }

    Ok(())
}
