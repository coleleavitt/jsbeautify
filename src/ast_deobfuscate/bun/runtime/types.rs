use rustc_hash::FxHashMap;

/// Defines [`BunHelper`] from one `Variant => "canonical name"` list, so
/// [`BunHelper::ALL`] and [`BunHelper::canonical`] cover every variant.
macro_rules! bun_helpers {
    ($($variant:ident => $canonical:literal,)*) => {
        /// The runtime helpers esbuild and Bun emit, by their canonical names (the
        /// rename targets); esbuild's are listed in `internal/runtime/runtime.go`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum BunHelper {
            $($variant,)*
        }

        impl BunHelper {
            /// Every helper.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            #[must_use]
            pub const fn canonical(&self) -> &'static str {
                match self {
                    $(Self::$variant => $canonical,)*
                }
            }
        }
    };
}

bun_helpers! {
    Create => "__create",
    GetProtoOf => "__getProtoOf",
    DefProp => "__defProp",
    GetOwnPropNames => "__getOwnPropNames",
    GetOwnPropDesc => "__getOwnPropDesc",
    HasOwnProp => "__hasOwnProp",
    AccessProp => "__accessProp",
    CopyProps => "__copyProps",
    ReExport => "__reExport",
    CommonJs => "__commonJS",
    Esm => "__esm",
    ToEsm => "__toESM",
    ToCommonJs => "__toCommonJS",
    Export => "__export",
    Name => "__name",
    Freeze => "__freeze",
    DefProps => "__defProps",
    GetOwnPropDescs => "__getOwnPropDescs",
    GetOwnPropSymbols => "__getOwnPropSymbols",
    PropIsEnum => "__propIsEnum",
    ReflectGet => "__reflectGet",
    ReflectSet => "__reflectSet",
    Pow => "__pow",
    DefNormalProp => "__defNormalProp",
    SpreadValues => "__spreadValues",
    SpreadProps => "__spreadProps",
    ObjRest => "__objRest",
    PublicField => "__publicField",
    TypeError => "__typeError",
    AccessCheck => "__accessCheck",
    PrivateIn => "__privateIn",
    PrivateGet => "__privateGet",
    PrivateAdd => "__privateAdd",
    PrivateSet => "__privateSet",
    PrivateMethod => "__privateMethod",
    EarlyAccess => "__earlyAccess",
    KnownSymbol => "__knownSymbol",
    Template => "__template",
    Async => "__async",
    Require => "__require",
    Glob => "__glob",
    RestKey => "__restKey",
    DecorateClass => "__decorateClass",
    DecorateParam => "__decorateParam",
    DecoratorStart => "__decoratorStart",
    DecoratorStrings => "__decoratorStrings",
    ExpectFn => "__expectFn",
    DecoratorContext => "__decoratorContext",
    DecoratorMetadata => "__decoratorMetadata",
    RunInitializers => "__runInitializers",
    DecorateElement => "__decorateElement",
    PrivateWrapper => "__privateWrapper",
    SuperGet => "__superGet",
    SuperSet => "__superSet",
    SuperWrapper => "__superWrapper",
    Await => "__await",
    AsyncGenerator => "__asyncGenerator",
    YieldStar => "__yieldStar",
    ForAwait => "__forAwait",
    ToBinaryNode => "__toBinaryNode",
    ToBinary => "__toBinary",
    Using => "__using",
    CallDispose => "__callDispose",
}

impl BunHelper {
    /// The helper whose [`Self::canonical`] name is `name`.
    #[must_use]
    pub fn from_canonical(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|helper| helper.canonical() == name)
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
