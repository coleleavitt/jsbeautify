//! esbuild Runtime Helper Detection and Annotation
//!
//! Detects and annotates esbuild-specific runtime helpers:
//! - `__commonJS` / `__commonJSMin` (B) - Lazy CJS module wrapper
//! - `__esm` / `__esmMin` (L) - Lazy ESM initializer
//! - `__export` (G8) - Named exports with property descriptors
//! - `__toESM` (Y6) - CJS→ESM conversion
//! - `__toCommonJS` (bq) - ESM→CJS wrapper
//! - Object aliases (HI6 = Object.defineProperty, etc.)
//!
//! Also handles Anthropic-specific modifications:
//! - `__export` with `configurable: true` and `set:` accessor
//! - `WeakMap` caching in `__toESM` / `__toCommonJS` (dual-cache oz5/az5, single-cache jH7)
//! - Getter/setter bind helpers (MH7, sz5, tz5)

mod annotation_names;
mod annotations;
mod arrow_detection;
mod collector;
mod function_detection;
mod object_aliases;
mod types;

mod patterns;

#[cfg(test)]
mod tests;

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub use annotations::annotate_esbuild_modules;
pub use collector::EsbuildHelperCollector;
pub use types::{
    AnthropicModification, BundleType, EsbuildHelperInfo, EsbuildHelperKind, ObjectAliasKind, SupportHelperKind,
    WeakMapCacheKind,
};

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;
