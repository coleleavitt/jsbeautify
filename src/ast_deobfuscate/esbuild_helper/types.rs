/// Detected bundle type based on helper patterns
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BundleType {
    /// Standard esbuild without modifications
    Standard,
    /// Anthropic-modified esbuild (Claude Code, etc.)
    Anthropic,
    /// Unable to determine
    #[default]
    Unknown,
}

/// Anthropic-specific modifications to standard esbuild helpers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicModification {
    /// `__export` with `configurable: true` and `set:` accessor
    ConfigurableExport,
    /// `WeakMap` caching in module conversion helpers
    WeakMapCache(WeakMapCacheKind),
    /// Getter bind helper using `this[param]` pattern
    GetterBind,
    /// Setter bind helper for configurable exports
    SetterBind,
    /// Identity function `(q) => q` used by setter bind
    IdentityFunction,
}

/// Types of `WeakMap` caches used by Anthropic's esbuild fork
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeakMapCacheKind {
    /// Cache for `__toESM` in node mode (K=true) - e.g., `oz5`
    ToEsmNodeMode,
    /// Cache for `__toESM` in browser mode (K=false) - e.g., `az5`
    ToEsmBrowserMode,
    /// Cache for `__toCommonJS` - e.g., `jH7`
    ToCommonJs,
}

/// Anthropic-only support helpers (not in standard esbuild)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportHelperKind {
    /// Getter bind: `function(q) { return this[q] }` - used for efficient property access
    GetterBind,
    /// Identity function: `(q) => q` - used by setter bind
    Identity,
    /// Setter bind: `function(q, K) { this[q] = ... }` - used by configurable export
    SetterBind,
}

/// Types of esbuild runtime helpers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EsbuildHelperKind {
    /// `(cb, mod) => () => (mod || cb((mod = {exports: {}}).exports, mod), mod.exports)`
    CommonJs,
    /// `(fn, res) => () => (fn && (res = fn(fn = 0)), res)`
    Esm,
    /// `for (name in all) __defProp(target, name, {get: all[name], enumerable: true})`
    Export,
    /// CJS→ESM conversion with prototype chain
    ToEsm,
    /// ESM→CJS wrapper with `__esModule` flag
    ToCommonJs,
    /// Property copier for re-exports
    CopyProps,
    /// `export * from` implementation
    ReExport,
    /// `var __defProp = Object.defineProperty`
    ObjectAlias(ObjectAliasKind),
    /// Anthropic-only support helpers (MH7, sz5, tz5)
    SupportHelper(SupportHelperKind),
    /// `WeakMap` cache variable declaration
    WeakMapCache(WeakMapCacheKind),

    /// `(mod && mod.__esModule) ? mod : { "default": mod }`
    ImportDefault,
    /// `if (mod && mod.__esModule) return mod; ... __setModuleDefault(result, mod)`
    ImportStar,
    /// `Object.defineProperty(o, k2, { enumerable: true, get: function() { return m[k]; } })`
    CreateBinding,
    /// `for (var p in m) if (p !== "default" && !hasOwnProperty.call(exports, p)) __createBinding(...)`
    ExportStar,

    /// Checks for `Symbol.dispose` or `Symbol.for("Symbol.dispose")`
    AddDisposableResource,
    /// `SuppressedError` constructor, resource stack management
    DisposeResources,

    /// `new (P || (P = Promise))(function (resolve, reject) { ... step(generator.next(value)) })`
    Awaiter,
    /// `{ label: 0, sent: function() {}, trys: [], ops: [] }` state machine
    Generator,
    /// `Symbol.asyncIterator`, `q = []`, async iteration protocol
    AsyncGenerator,
    /// Simpler than `__awaiter`, uses Promise.resolve chaining
    Async,

    /// `if (kind === "a" && !f) throw new TypeError(...); return kind === "m" ? f : ...`
    PrivateGet,
    /// `if (kind === "m") throw new TypeError("Private method is not writable")`
    PrivateSet,
    /// `if (state.has(obj)) throw new TypeError(...); state.add(obj)`
    PrivateAdd,
    /// Returns the private method bound to the instance
    PrivateMethod,
    /// `typeof state === "function" ? receiver === state : state.has(receiver)`
    PrivateIn,
    /// Simple field assignment with defineProperty
    PublicField,
    /// Private field getter/setter wrapper
    PrivateWrapper,

    /// `Math.pow(base, exp)` polyfill for `**` operator
    Pow,
    /// `Object.defineProperty(f, "name", { value: name, configurable: true })`
    Name,
    /// Fallback `require()` with Proxy support for missing modules
    Require,
    /// `import.meta.glob` handler
    Glob,
    /// Object rest key normalization for `...rest` patterns
    RestKey,
    /// `{ ...rest } = obj` object rest extraction
    ObjRest,

    /// `for (k in b) if (__hasOwnProp.call(b, k)) a[k] = b[k]` with symbol support
    SpreadValues,
    /// Spread via Object.defineProperties with descriptors
    SpreadProps,
    /// `{ a, ...rest } = obj` rest property extraction (tslib)
    Rest,
    /// Legacy spread: `ar.concat(__read(arguments[i]))`
    Spread,
    /// `Array(s)` concatenation spread
    SpreadArrays,
    /// Modern spread with `Array.prototype.slice.call`
    SpreadArray,

    /// `extendStatics(d, b); d.prototype = Object.create(b.prototype)`
    Extends,
    /// `Object.assign || function(t) { for (var p in s) ... }`
    Assign,

    /// `Symbol.iterator` iteration protocol
    Values,
    /// `while ((n === void 0 || n-- > 0) && !(r = i.next()).done) ar.push(r.value)`
    Read,
    /// `this instanceof __await ? (this.v = v, this) : new __await(v)`
    Await,
    /// `yield* o` async delegation wrapper
    AsyncDelegator,
    /// `Symbol.asyncIterator` async iteration
    AsyncValues,
    /// `yield*` in async generators
    YieldStar,
    /// `for await (const x of y)` lowering
    ForAwait,

    /// `Reflect.get(target, prop, receiver)` super property access
    SuperGet,
    /// `Reflect.set(target, prop, value, receiver)` super property assignment
    SuperSet,
    /// Combined super getter/setter wrapper
    SuperWrapper,
    /// Tagged template literal: `Object.defineProperty(cooked, "raw", { value: raw })`
    Template,
    /// `{ raw: [...] }` template object creation (tslib)
    MakeTemplateObject,

    /// `atob` / Base64 decode to `Uint8Array` (browser)
    ToBinary,
    /// `Buffer.from(base64, "base64")` (Node.js)
    ToBinaryNode,
    /// `typeof x === "symbol" ? x : "".concat(x)` property key normalization
    PropKey,
    /// `Object.defineProperty(f, "name", ...)` function name setter
    SetFunctionName,
    /// Temporal dead zone error: `throw new ReferenceError("...")`
    EarlyAccess,

    /// `decorators.reduceRight((o, d) => d(o), target)` decorator application
    Decorate,
    /// `function(paramIndex, decorator) { return (target, key) => decorator(...) }`
    Param,
    /// ES decorators: `context.addInitializer`, `Reflect.decorate`
    EsDecorate,
    /// `Reflect.metadata(metadataKey, metadataValue)`
    Metadata,
    /// Class/method/field decorator application (esbuild)
    DecorateClass,
    /// Parameter decorator wrapper (esbuild)
    DecorateParam,
    /// JS decorator metadata initialization
    DecoratorStart,
    /// Attach metadata to decorated element
    DecoratorMetadata,
    /// Execute decorator initializers
    RunInitializers,
    /// JS decorator element processing
    DecorateElement,
}

/// Types of Object method aliases
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectAliasKind {
    DefineProperty,
    GetOwnPropertyNames,
    GetOwnPropertyDescriptor,
    GetPrototypeOf,
    Create,
    HasOwnProperty,
    PropertyIsEnumerable,
}

/// Information about a detected esbuild helper
#[derive(Debug, Clone)]
pub struct EsbuildHelperInfo {
    pub mangled_name: String,
    pub kind: EsbuildHelperKind,
    pub line_number: u32,
    pub anthropic_modifications: Vec<AnthropicModification>,
}

impl EsbuildHelperInfo {
    #[must_use]
    pub const fn new(mangled_name: String, kind: EsbuildHelperKind, line_number: u32) -> Self {
        Self {
            mangled_name,
            kind,
            line_number,
            anthropic_modifications: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_modification(mut self, modification: AnthropicModification) -> Self {
        self.anthropic_modifications.push(modification);
        self
    }

    #[must_use]
    pub const fn is_anthropic_modified(&self) -> bool {
        !self.anthropic_modifications.is_empty()
    }
}
