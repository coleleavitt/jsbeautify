use super::{Cli, align, analysis, directory, recon, scan, sources, wordlist};
use clap::Parser;
use js_beautify_rs::tokenizer::Tokenizer;
use js_beautify_rs::webpack_module_extractor::ModuleExtractor;
use js_beautify_rs::{Options, beautify};
use std::fs;
use std::io::{self, Read};

pub(super) fn main() {
    #[cfg(feature = "dhat-heap")]
    let _dhat = dhat::Profiler::new_heap();

    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    if cli.serve {
        return recon::run_serve(&cli);
    }
    if let Some(domain) = cli.crtsh.clone() {
        return recon::run_crtsh(&cli, &domain);
    }
    if let Some(domain) = cli.recon.clone() {
        return recon::run_recon(&cli, &domain);
    }
    if cli.split_sources.is_some() {
        return sources::run_split_sources(&cli);
    }
    if cli.reconstruct_sourcemap.is_some() {
        return sources::run_reconstruct_sourcemap(&cli);
    }
    if cli.relationships {
        return sources::run_relationships(&cli);
    }
    if let Some(ref map_path) = cli.reverse_sourcemap {
        return sources::run_reverse_sourcemap(&cli, map_path);
    }
    if cli.dir {
        return directory::run_directory_scan(&cli);
    }

    let mut code = input_source(&cli)?;
    let mut options = options_from_cli(&cli);
    if options.extract_modules {
        return extract_modules(&code, &options);
    }

    let sourcemap_rename = (cli.rename_vars || cli.rename_llm) && cli.sourcemap.is_some() && cli.align_with.is_none();
    if sourcemap_rename {
        let sm = fs::read_to_string(cli.sourcemap.as_ref().expect("sourcemap present"))?;
        eprintln!("[RENAME] Applying sourcemap original names...");
        code = js_beautify_rs::ast_deobfuscate::sourcemap_rename::apply_sourcemap_names(&code, &sm)?;
    }
    if !sourcemap_rename
        && (cli.align_with.is_some() || cli.sourcemap.is_some() || cli.names_json.is_some() || cli.bun_extract)
    {
        options.skip_annotations = true;
        return align::run_cross_version_align(
            cli.sourcemap.as_ref(),
            cli.names_json.as_ref(),
            cli.align_with.as_ref(),
            cli.align_output.as_ref(),
            cli.output.as_ref(),
            &code,
            &options,
            cli.raw,
            cli.bun_extract,
        );
    }
    if analysis::is_analysis_requested(&cli) {
        return analysis::run_analysis(&cli, &code);
    }
    if cli.wordlist {
        return wordlist::run_wordlist(&cli, &code);
    }
    if !cli.custom_regex.is_empty() {
        return scan::run_custom_regex(&cli, &code);
    }
    if cli.analyze_html {
        return scan::run_analyze_html(&cli, &code);
    }
    if cli.discover_nextjs {
        return scan::run_nextjs_discovery(&cli, &code);
    }
    if let Some(ref old_path) = cli.diff_with {
        return scan::run_diff(&cli, &code, old_path);
    }

    let mut beautified = beautify(&code, &options)?;
    if beautified.trim().is_empty() && !code.trim().is_empty() {
        eprintln!("[jsbeautify] empty output after processing; emitting original source unchanged");
        beautified = code;
    }
    if cli.rename_llm {
        let overrides = js_beautify_rs::llm::LlmOverrides {
            url: cli.llm_url.clone(),
            model: cli.llm_model.clone(),
            key: cli.llm_key.clone(),
        };
        match js_beautify_rs::ast_deobfuscate::llm_rename::OpenAiCompatSuggester::from_overrides(&overrides) {
            Some(suggester) => {
                eprintln!("[RENAME] Querying LLM for residual minified names...");
                beautified = js_beautify_rs::ast_deobfuscate::llm_rename::apply_llm_names(&beautified, &suggester);
            }
            None => eprintln!(
                "[RENAME] --rename-llm set but no endpoint configured \
                 (set OPENAI_API_KEY, or --llm-url / JSBEAUTIFY_LLM_URL); skipping LLM tier."
            ),
        }
    }
    output_beautified(&cli, &beautified)
}

fn input_source(cli: &Cli) -> Result<String, Box<dyn std::error::Error>> {
    if cli.input == "-" {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        Ok(buffer)
    } else {
        Ok(fs::read_to_string(&cli.input)?)
    }
}

fn options_from_cli(cli: &Cli) -> Options {
    let mut options = Options {
        deobfuscate: cli.deobfuscate || cli.rename_vars || cli.rename_llm,
        readable_names: cli.rename_vars || cli.rename_llm,
        split_chunks: cli.split_chunks,
        extract_modules: cli.extract_modules,
        generate_source_map: cli.source_maps,
        indent_with_tabs: cli.indent_with_tabs,
        ..Options::default()
    };
    if let Some(dir) = &cli.chunk_dir {
        options.chunk_dir.clone_from(dir);
    }
    if let Some(path) = &cli.chunk_map {
        options.chunk_map_output = Some(path.clone());
    }
    if let Some(dir) = &cli.module_dir {
        options.module_dir.clone_from(dir);
    }
    if let Some(path) = &cli.dependency_graph {
        options.dependency_graph = Some(path.clone());
    }
    if let Some(size) = cli.indent_size {
        options.indent_size = size;
    }
    options
}

fn extract_modules(code: &str, options: &Options) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[WEBPACK] Extracting modules...");
    let mut tokenizer = Tokenizer::new(code);
    let tokens = tokenizer.tokenize()?;
    let mut extractor = ModuleExtractor::new();
    extractor.extract_modules(&tokens)?;
    extractor.extract_dependencies(&tokens)?;
    eprintln!("[WEBPACK] Found {} modules", extractor.module_count());
    extractor.write_modules(&tokens, &options.module_dir)?;
    eprintln!("[WEBPACK] Modules written to {}", options.module_dir.display());
    if let Some(graph_path) = &options.dependency_graph {
        extractor.generate_dependency_graph(graph_path)?;
        eprintln!("[WEBPACK] Dependency graph written to {}", graph_path.display());
    }
    Ok(())
}

fn output_beautified(cli: &Cli, beautified: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(output_path) = &cli.output {
        fs::write(output_path, beautified)?;
        eprintln!("Beautified code written to {output_path}");
    } else {
        println!("{beautified}");
    }
    Ok(())
}
