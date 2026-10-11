// Heap profiling: with `--features dhat-heap`, dhat replaces the global
// allocator and records every allocation's call site (see Cargo.toml).
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod align;
mod config;
mod runner;
mod sources;

/// A fast JavaScript beautifier and deobfuscator powered by oxc.
///
/// With no subcommand, beautifies (and optionally deobfuscates) a single file
/// or stdin. Bundle-splitting and cross-version alignment live under
/// subcommands so the default surface stays small.
#[derive(Parser, Debug)]
#[command(name = "jsbeautify", version, about, long_about = None)]
struct Cli {
    /// Print version.
    #[arg(short = 'v', action = clap::ArgAction::Version, hide = true)]
    version_short: (),

    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    beautify: BeautifyArgs,
}

/// Default (no-subcommand) beautify/deobfuscate options.
#[allow(clippy::struct_excessive_bools)]
#[derive(clap::Args, Debug)]
struct BeautifyArgs {
    /// Input JavaScript files, or "-" for stdin [default: -]. Several files
    /// are beautified in place, as with --replace.
    #[arg(value_name = "FILE")]
    inputs: Vec<String>,

    /// Input file; may be repeated (same as a positional FILE).
    #[arg(short = 'f', long = "file", value_name = "FILE")]
    files: Vec<String>,

    /// Write output to a file instead of stdout.
    #[arg(short, long, value_name = "FILE", alias = "outfile")]
    output: Option<String>,

    /// Write output in place, replacing each input file.
    #[arg(short, long)]
    replace: bool,

    /// Do not report written files on stderr.
    #[arg(short, long)]
    quiet: bool,

    /// Enable AST-based deobfuscation (multi-phase pipeline).
    #[arg(short, long)]
    deobfuscate: bool,

    /// Infer readable names for minified locals (loop→i/j/k, catch→error,
    /// require→module, new Foo→instance) instead of v{n} slots.
    /// Implies --deobfuscate.
    #[arg(long)]
    rename_vars: bool,

    /// After heuristic renaming, ask an LLM to name residual minified locals
    /// (humanify pattern). Configure the endpoint via `OPENAI_API_KEY` /
    /// `OPENAI_BASE_URL` / `OPENAI_MODEL` or `JSBEAUTIFY_LLM_URL` / `_MODEL` / `_KEY`.
    /// Implies --rename-vars.
    #[arg(long)]
    rename_llm: bool,

    #[command(flatten)]
    format: config::FormatArgs,

    /// Apply original variable names from a sourcemap (.map) file.
    #[arg(long, value_name = "FILE")]
    sourcemap: Option<PathBuf>,

    /// Apply full name mappings from a names.json file.
    #[arg(long, value_name = "FILE")]
    names_json: Option<PathBuf>,

    /// Recover readable names from Bun bundle patterns (MR exports, this.name,
    /// displayName). Best combined with --sourcemap.
    #[arg(long)]
    bun_extract: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Extract webpack modules into separate files.
    ExtractModules(ExtractModulesArgs),

    /// Split webpack chunks into separate files.
    SplitChunks(SplitChunksArgs),

    /// Reconstruct the original source tree from a bundle — via source maps
    /// when present, or synthetic module/function boundaries when absent.
    SplitSources(SplitSourcesArgs),

    /// Rebuild original sources from a standalone .js.map file.
    Reconstruct(ReconstructArgs),

    /// Align two bundle versions for a stable diff, recovering names via a
    /// sourcemap / names.json.
    Align(AlignArgs),

    /// Statically inspect a JavaScript-hosted VM and embedded WebAssembly.
    /// Never executes JavaScript or instantiates WebAssembly.
    AnalyzeVm(AnalyzeVmArgs),

    #[command(about = "Print the program as ESTree JSON (oxc's serializer)")]
    Ast(InspectArgs),

    /// Print oxc's semantic symbol table as JSON: scopes, symbols with
    /// their references, and unresolved (global) references.
    Scope(InspectArgs),

    /// Print oxc's semantic control-flow graph.
    Cfg(CfgArgs),
}

#[derive(clap::Args, Debug)]
struct InspectArgs {
    /// Input JavaScript file, or "-" for stdin.
    #[arg(value_name = "FILE", default_value = "-")]
    input: String,

    /// Write output to a file instead of stdout.
    #[arg(short, long, value_name = "FILE")]
    output: Option<String>,

    /// Parse as TypeScript with JSX instead of module JavaScript.
    #[arg(long)]
    ts: bool,
}

#[derive(clap::Args, Debug)]
struct CfgArgs {
    #[command(flatten)]
    inspect: InspectArgs,

    /// Only the functions with this name (declaration, or the variable,
    /// property, method or assignment target it is bound to).
    #[arg(long, value_name = "NAME")]
    function: Option<String>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = CfgFormat::Dot)]
    format: CfgFormat,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum CfgFormat {
    Dot,
    Json,
}

#[derive(clap::Args, Debug)]
struct AnalyzeVmArgs {
    /// Input JavaScript file, or "-" for stdin.
    #[arg(value_name = "FILE", default_value = "-")]
    input: String,

    /// Write the JSON report to a file instead of stdout.
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Extract literal embedded WebAssembly modules into this directory.
    /// Existing module files are never overwritten.
    #[arg(long, value_name = "DIR")]
    extract_wasm: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
struct ExtractModulesArgs {
    /// Input bundle, or "-" for stdin.
    #[arg(value_name = "FILE", default_value = "-")]
    input: String,

    /// Directory for module output [default: ./modules].
    #[arg(short, long, value_name = "DIR")]
    output: Option<PathBuf>,

    /// Generate a dependency graph in DOT format.
    #[arg(long, value_name = "FILE")]
    graph: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
struct SplitChunksArgs {
    /// Input bundle, or "-" for stdin.
    #[arg(value_name = "FILE", default_value = "-")]
    input: String,

    /// Directory for chunk output [default: ./chunks].
    #[arg(short, long, value_name = "DIR")]
    output: Option<PathBuf>,

    /// Write chunk metadata to a JSON file.
    #[arg(long, value_name = "FILE")]
    chunk_map: Option<PathBuf>,

    /// Generate a source map for each split chunk.
    #[arg(long)]
    source_maps: bool,
}

#[derive(clap::Args, Debug)]
struct SplitSourcesArgs {
    /// Input bundle, or "-" for stdin.
    #[arg(value_name = "FILE", default_value = "-")]
    input: String,

    /// Output directory (files land under <DIR>/reversed/).
    #[arg(short, long, value_name = "DIR", default_value = "./out")]
    output: String,
}

#[derive(clap::Args, Debug)]
struct ReconstructArgs {
    /// Input .js.map sourcemap file, or "-" for stdin.
    #[arg(value_name = "MAP", default_value = "-")]
    input: String,

    /// Output directory (files land under <DIR>/reversed/).
    #[arg(short, long, value_name = "DIR", default_value = "./out")]
    output: String,
}

#[derive(clap::Args, Debug)]
struct AlignArgs {
    /// First (reference) bundle.
    #[arg(value_name = "V1")]
    input: String,

    /// Second bundle to align against V1.
    #[arg(value_name = "V2")]
    align_with: PathBuf,

    /// Sourcemap for recovering original names.
    #[arg(long, value_name = "FILE")]
    sourcemap: Option<PathBuf>,

    /// names.json with full name mappings.
    #[arg(long, value_name = "FILE")]
    names_json: Option<PathBuf>,

    /// Output path for the aligned V1 (stdout if omitted).
    #[arg(short, long, value_name = "FILE")]
    output: Option<String>,

    /// Output path for the aligned V2.
    #[arg(long, value_name = "FILE")]
    align_output: Option<PathBuf>,

    /// Skip beautification, output raw aligned code.
    #[arg(long)]
    raw: bool,
}

pub fn main() {
    runner::main();
}
