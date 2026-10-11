use super::{
    AlignArgs, AnalyzeVmArgs, BeautifyArgs, CfgFormat, Cli, Command, ExtractModulesArgs, InspectArgs, ReconstructArgs,
    SplitChunksArgs, SplitSourcesArgs, align, sources,
};
use clap::Parser;
use js_beautify_rs::inspect::{self, Inspection};
use js_beautify_rs::tokenizer::Tokenizer;
use js_beautify_rs::webpack_module_extractor::ModuleExtractor;
use js_beautify_rs::{Options, beautify};
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

type Fallible = Result<(), Box<dyn std::error::Error>>;

pub(super) fn main() {
    #[cfg(feature = "dhat-heap")]
    let _dhat = dhat::Profiler::new_heap();

    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Fallible {
    let cli = Cli::parse();
    match &cli.command {
        Some(Command::ExtractModules(a)) => run_extract_modules(a),
        Some(Command::SplitChunks(a)) => run_split_chunks(a),
        Some(Command::SplitSources(a)) => run_split_sources(a),
        Some(Command::Reconstruct(a)) => run_reconstruct(a),
        Some(Command::Align(a)) => run_align(a),
        Some(Command::AnalyzeVm(a)) => run_analyze_vm(a),
        Some(Command::Ast(a)) => run_inspect(a, |source| inspect::ast::estree(source, a.ts), Clone::clone),
        Some(Command::Scope(a)) => run_inspect(
            a,
            |source| inspect::scope::symbol_table(source, a.ts),
            |report| serde_json::to_string_pretty(report).expect("scope report serializes"),
        ),
        Some(Command::Cfg(a)) => run_inspect(
            &a.inspect,
            |source| inspect::cfg::control_flow(source, a.inspect.ts, a.function.as_deref()),
            |graph| match a.format {
                CfgFormat::Dot => graph.to_dot(),
                CfgFormat::Json => serde_json::to_string_pretty(graph).expect("CFG serializes"),
            },
        ),
        None => run_beautify(&cli.beautify),
    }
}

fn read_input(input: &str) -> Result<String, Box<dyn std::error::Error>> {
    if input == "-" {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        Ok(buffer)
    } else {
        Ok(fs::read_to_string(input)?)
    }
}

fn read_vm_input_bounded(input: &str) -> Result<String, Box<dyn std::error::Error>> {
    let limit = js_beautify_rs::vm_analysis::AnalysisLimits::default().max_source_bytes;
    read_input_bounded(input, limit, "analyze-vm")
}

fn read_input_bounded(input: &str, limit: usize, command: &str) -> Result<String, Box<dyn std::error::Error>> {
    let reader: Box<dyn Read> = if input == "-" {
        Box::new(io::stdin())
    } else {
        let metadata = fs::metadata(input)?;
        if metadata.len() > limit as u64 {
            return Err(format!("{command} input exceeds {limit} byte limit").into());
        }
        Box::new(fs::File::open(input)?)
    };
    let mut reader = reader;
    let mut bytes = Vec::new();
    let mut chunk = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        if read > limit.saturating_sub(bytes.len()) {
            return Err(format!("{command} input exceeds {limit} byte limit").into());
        }
        bytes.try_reserve_exact(read)?;
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(String::from_utf8(bytes)?)
}

fn write_output(output: Option<&String>, text: &str) -> Fallible {
    if let Some(output_path) = output {
        fs::write(output_path, text)?;
        eprintln!("Output written to {output_path}");
    } else {
        println!("{text}");
    }
    Ok(())
}

fn run_analyze_vm(args: &AnalyzeVmArgs) -> Fallible {
    let source = read_vm_input_bounded(&args.input)?;
    let artifact = js_beautify_rs::vm_analysis::analyze_artifact(&source);
    let json = serde_json::to_string_pretty(&artifact.report)?;
    if let Some(path) = &args.output {
        fs::write(path, format!("{json}\n"))?;
        eprintln!("VM analysis written to {}", path.display());
    } else {
        println!("{json}");
    }
    if let Some(directory) = &args.extract_wasm {
        fs::create_dir_all(directory)?;
        for (index, module) in artifact.extracted_wasm.iter().enumerate() {
            let path = directory.join(format!("module-{index:03}.wasm"));
            let mut output = match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(output) => output,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    eprintln!("Skipped existing WebAssembly module {}", path.display());
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            output.write_all(&module.bytes)?;
            eprintln!("Extracted WebAssembly module to {}", path.display());
        }
    }
    Ok(())
}

fn run_inspect<T>(
    args: &InspectArgs,
    inspect: impl FnOnce(&str) -> Result<Inspection<T>, inspect::InspectError>,
    render: impl FnOnce(&T) -> String,
) -> Fallible {
    let source = read_input(&args.input)?;
    let inspection = inspect(&source)?;
    for diagnostic in &inspection.diagnostics {
        eprintln!("{diagnostic}");
    }
    write_output(args.output.as_ref(), render(&inspection.report).trim_end())
}

fn run_beautify(b: &BeautifyArgs) -> Fallible {
    let mut code = read_input(&b.input)?;
    let mut options = Options {
        deobfuscate: b.deobfuscate || b.rename_vars || b.rename_llm,
        readable_names: b.rename_vars || b.rename_llm,
        indent_with_tabs: b.tabs,
        ..Options::default()
    };
    if let Some(size) = b.indent {
        options.indent_size = size;
    }

    let sourcemap_rename = (b.rename_vars || b.rename_llm) && b.sourcemap.is_some();
    if sourcemap_rename {
        let sm = fs::read_to_string(b.sourcemap.as_ref().expect("sourcemap present"))?;
        eprintln!("[RENAME] Applying sourcemap original names...");
        code = js_beautify_rs::ast_deobfuscate::sourcemap_rename::apply_sourcemap_names(&code, &sm)?;
    }
    if !sourcemap_rename && (b.sourcemap.is_some() || b.names_json.is_some() || b.bun_extract) {
        options.skip_annotations = true;
        return align::run_cross_version_align(
            b.sourcemap.as_ref(),
            b.names_json.as_ref(),
            None,
            None,
            b.output.as_ref(),
            &code,
            &options,
            false,
            b.bun_extract,
        );
    }

    let mut beautified = beautify(&code, &options)?;
    if beautified.trim().is_empty() && !code.trim().is_empty() {
        eprintln!("[jsbeautify] empty output after processing; emitting original source unchanged");
        beautified = code;
    }
    if b.rename_llm {
        let overrides = js_beautify_rs::llm::LlmOverrides::default();
        match js_beautify_rs::ast_deobfuscate::llm_rename::OpenAiCompatSuggester::from_overrides(&overrides) {
            Some(suggester) => {
                eprintln!("[RENAME] Querying LLM for residual minified names...");
                beautified = js_beautify_rs::ast_deobfuscate::llm_rename::apply_llm_names(&beautified, &suggester);
            }
            None => eprintln!(
                "[RENAME] --rename-llm set but no endpoint configured \
                 (set OPENAI_API_KEY, OPENAI_BASE_URL, or JSBEAUTIFY_LLM_URL); skipping LLM tier."
            ),
        }
    }
    write_output(b.output.as_ref(), &beautified)
}

fn run_extract_modules(a: &ExtractModulesArgs) -> Fallible {
    let code = read_input(&a.input)?;
    let module_dir = a.output.clone().unwrap_or_else(|| PathBuf::from("./modules"));
    eprintln!("[WEBPACK] Extracting modules...");
    let mut tokenizer = Tokenizer::new(&code);
    let tokens = tokenizer.tokenize()?;
    let mut extractor = ModuleExtractor::new();
    extractor.extract_modules(&tokens)?;
    extractor.extract_dependencies(&tokens)?;
    eprintln!("[WEBPACK] Found {} modules", extractor.module_count());
    extractor.write_modules(&tokens, &module_dir)?;
    eprintln!("[WEBPACK] Modules written to {}", module_dir.display());
    if let Some(graph_path) = &a.graph {
        extractor.generate_dependency_graph(graph_path)?;
        eprintln!("[WEBPACK] Dependency graph written to {}", graph_path.display());
    }
    Ok(())
}

fn run_split_chunks(a: &SplitChunksArgs) -> Fallible {
    let code = read_input(&a.input)?;
    let mut options = Options {
        split_chunks: true,
        generate_source_map: a.source_maps,
        ..Options::default()
    };
    if let Some(dir) = &a.output {
        options.chunk_dir.clone_from(dir);
    }
    if let Some(path) = &a.chunk_map {
        options.chunk_map_output = Some(path.clone());
    }
    let out = beautify(&code, &options)?;
    print!("{out}");
    Ok(())
}

fn run_split_sources(a: &SplitSourcesArgs) -> Fallible {
    let bundle = read_input(&a.input)?;
    let bundle_path = (a.input != "-").then(|| PathBuf::from(&a.input));
    sources::run_split_sources(&bundle, bundle_path.as_deref(), &a.output)
}

fn run_reconstruct(a: &ReconstructArgs) -> Fallible {
    let map_json = read_input(&a.input)?;
    sources::run_reconstruct_sourcemap(&map_json, &a.output)
}

fn run_align(a: &AlignArgs) -> Fallible {
    let code = read_input(&a.input)?;
    let options = Options {
        skip_annotations: true,
        ..Options::default()
    };
    align::run_cross_version_align(
        a.sourcemap.as_ref(),
        a.names_json.as_ref(),
        Some(&a.align_with),
        a.align_output.as_ref(),
        a.output.as_ref(),
        &code,
        &options,
        a.raw,
        false,
    )
}
