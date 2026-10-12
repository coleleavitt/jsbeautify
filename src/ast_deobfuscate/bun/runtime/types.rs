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
    RestKey,
    DecorateClass,
    DecorateParam,
    DecoratorStart,
    DecoratorStrings,
    ExpectFn,
    DecoratorContext,
    DecoratorMetadata,
    RunInitializers,
    DecorateElement,
    PrivateWrapper,
    SuperGet,
    SuperSet,
    SuperWrapper,
    Await,
    AsyncGenerator,
    YieldStar,
    ForAwait,
    ToBinaryNode,
    ToBinary,
    Using,
    CallDispose,
}

impl BunHelper {
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
            Self::RestKey => "__restKey",
            Self::DecorateClass => "__decorateClass",
            Self::DecorateParam => "__decorateParam",
            Self::DecoratorStart => "__decoratorStart",
            Self::DecoratorStrings => "__decoratorStrings",
            Self::ExpectFn => "__expectFn",
            Self::DecoratorContext => "__decoratorContext",
            Self::DecoratorMetadata => "__decoratorMetadata",
            Self::RunInitializers => "__runInitializers",
            Self::DecorateElement => "__decorateElement",
            Self::PrivateWrapper => "__privateWrapper",
            Self::SuperGet => "__superGet",
            Self::SuperSet => "__superSet",
            Self::SuperWrapper => "__superWrapper",
            Self::Await => "__await",
            Self::AsyncGenerator => "__asyncGenerator",
            Self::YieldStar => "__yieldStar",
            Self::ForAwait => "__forAwait",
            Self::ToBinaryNode => "__toBinaryNode",
            Self::ToBinary => "__toBinary",
            Self::Using => "__using",
            Self::CallDispose => "__callDispose",
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
