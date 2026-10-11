use oxc_syntax::symbol::SymbolId;
use rustc_hash::FxHashMap;

pub struct DeobfuscateState {
    pub changed: bool,
    pub dispatchers: FxHashMap<String, DispatcherInfo>,
    pub string_arrays: FxHashMap<String, StringArrayInfo>,
    pub decoders: FxHashMap<String, DecoderInfo>,
}

impl DeobfuscateState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            changed: false,
            dispatchers: FxHashMap::default(),
            string_arrays: FxHashMap::default(),
            decoders: FxHashMap::default(),
        }
    }
}

impl Default for DeobfuscateState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct DispatcherInfo {
    pub var_name: String,
    pub symbol: Option<SymbolId>,
    pub functions: FxHashMap<String, FunctionInfo>,
}

#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub key: String,
    pub return_value: Option<ReturnValue>,
}

#[derive(Debug, Clone)]
pub enum ReturnValue {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone)]
pub struct StringArrayInfo {
    pub var_name: String,
    pub strings: Vec<String>,
    pub rotated: bool,
    pub rotation_count: usize,
}

#[derive(Debug, Clone)]
pub struct DecoderInfo {
    pub function_name: String,
    pub array_name: String,
    pub offset: i32,
    pub offset_operation: OffsetOperation,
    pub decoder_type: DecoderType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetOperation {
    None,
    Subtract,
    Add,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecoderType {
    Simple,
    Base64,
    Xor { key: Vec<u8> },
    Rc4 { key: Vec<u8> },
}
