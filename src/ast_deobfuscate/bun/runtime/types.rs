use rustc_hash::FxHashMap;

/// The runtime helpers esbuild and Bun emit, by their canonical names (the
/// rename targets); esbuild's are listed in `internal/runtime/runtime.go`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BunHelper {
    Create,
    GetProtoOf,
    DefProp,
    GetOwnPropNames,
    GetOwnPropDesc,
    HasOwnProp,
    AccessProp,
    CopyProps,
    ReExport,
    CommonJs,
    Esm,
    ToEsm,
    ToCommonJs,
    Export,
    Name,
    Freeze,
    DefProps,
    GetOwnPropDescs,
    GetOwnPropSymbols,
    PropIsEnum,
    ReflectGet,
    ReflectSet,
    Pow,
    DefNormalProp,
    SpreadValues,
    SpreadProps,
    ObjRest,
    PublicField,
    TypeError,
    AccessCheck,
    PrivateIn,
    PrivateGet,
    PrivateAdd,
    PrivateSet,
    PrivateMethod,
    EarlyAccess,
    KnownSymbol,
    Template,
    Async,
    Require,
    Glob,
}

impl BunHelper {
    pub const ALL: [Self; 41] = [
        Self::Create,
        Self::GetProtoOf,
        Self::DefProp,
        Self::GetOwnPropNames,
        Self::GetOwnPropDesc,
        Self::HasOwnProp,
        Self::AccessProp,
        Self::CopyProps,
        Self::ReExport,
        Self::CommonJs,
        Self::Esm,
        Self::ToEsm,
        Self::ToCommonJs,
        Self::Export,
        Self::Name,
        Self::Freeze,
        Self::DefProps,
        Self::GetOwnPropDescs,
        Self::GetOwnPropSymbols,
        Self::PropIsEnum,
        Self::ReflectGet,
        Self::ReflectSet,
        Self::Pow,
        Self::DefNormalProp,
        Self::SpreadValues,
        Self::SpreadProps,
        Self::ObjRest,
        Self::PublicField,
        Self::TypeError,
        Self::AccessCheck,
        Self::PrivateIn,
        Self::PrivateGet,
        Self::PrivateAdd,
        Self::PrivateSet,
        Self::PrivateMethod,
        Self::EarlyAccess,
        Self::KnownSymbol,
        Self::Template,
        Self::Async,
        Self::Require,
        Self::Glob,
    ];

    /// The helper whose [`Self::canonical`] name is `name`.
    #[must_use]
    pub fn from_canonical(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|helper| helper.canonical() == name)
    }

    #[must_use]
    pub const fn canonical(&self) -> &'static str {
        match self {
            Self::Create => "__create",
            Self::GetProtoOf => "__getProtoOf",
            Self::DefProp => "__defProp",
            Self::GetOwnPropNames => "__getOwnPropNames",
            Self::GetOwnPropDesc => "__getOwnPropDesc",
            Self::HasOwnProp => "__hasOwnProp",
            Self::AccessProp => "__accessProp",
            Self::CopyProps => "__copyProps",
            Self::ReExport => "__reExport",
            Self::CommonJs => "__commonJS",
            Self::Esm => "__esm",
            Self::ToEsm => "__toESM",
            Self::ToCommonJs => "__toCommonJS",
            Self::Export => "__export",
            Self::Name => "__name",
            Self::Freeze => "__freeze",
            Self::DefProps => "__defProps",
            Self::GetOwnPropDescs => "__getOwnPropDescs",
            Self::GetOwnPropSymbols => "__getOwnPropSymbols",
            Self::PropIsEnum => "__propIsEnum",
            Self::ReflectGet => "__reflectGet",
            Self::ReflectSet => "__reflectSet",
            Self::Pow => "__pow",
            Self::DefNormalProp => "__defNormalProp",
            Self::SpreadValues => "__spreadValues",
            Self::SpreadProps => "__spreadProps",
            Self::ObjRest => "__objRest",
            Self::PublicField => "__publicField",
            Self::TypeError => "__typeError",
            Self::AccessCheck => "__accessCheck",
            Self::PrivateIn => "__privateIn",
            Self::PrivateGet => "__privateGet",
            Self::PrivateAdd => "__privateAdd",
            Self::PrivateSet => "__privateSet",
            Self::PrivateMethod => "__privateMethod",
            Self::EarlyAccess => "__earlyAccess",
            Self::KnownSymbol => "__knownSymbol",
            Self::Template => "__template",
            Self::Async => "__async",
            Self::Require => "__require",
            Self::Glob => "__glob",
        }
    }
}

/// Map from mangled name → canonical helper.
pub type BunHelperMap = FxHashMap<String, BunHelper>;

/// A bundler identified from a runtime helper variant only it emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bundler {
    Bun,
    Esbuild,
}

impl Bundler {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bun => "Bun",
            Self::Esbuild => "esbuild",
        }
    }
}

/// The runtime helpers detected in a program and the bundler they identify.
#[derive(Debug, Default)]
pub struct BundleRuntime {
    pub helpers: BunHelperMap,
    pub bundler: Option<Bundler>,
}
