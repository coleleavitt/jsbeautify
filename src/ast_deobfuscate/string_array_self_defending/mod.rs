//! javascript-obfuscator's string array (the default `stringArray` output).
//!
//! The obfuscator emits three top-level pieces:
//!
//! 1. **Array**: a wrapper, `function W() { const a = [...]; W = function () {
//!    return a; }; return W(); }`, or, in older releases, `var a = [...]`.
//! 2. **Decoder**: `function D(i, k) { i = i - OFFSET; const s = W(); let v =
//!    s[i]; return v; }`, or `var D = function (i) { i = i - OFFSET; var v =
//!    a[i]; return v; }`, which older releases call with hex strings (`D('0x1')`).
//! 3. **Rotation IIFE** (optional): `push(shift())` on the array until a
//!    `parseInt(D(N))` checksum equals a target literal, or, in older
//!    releases, `while (--k)` from a counter.
//!
//! The pass matches the pieces through oxc symbols, solves the rotation
//! statically, and replaces each `D(<number literal>)` call, made directly
//! or through an alias of `D` (`const a = D`, at any depth), with the string
//! it returns. It runs before the control-flow unflattener, so that the
//! `'1|0|2'.split('|')` sequences stored in the array are literal there. When
//! no reference to the decoder is left, the rotation IIFE, whose only effect
//! is the array order, is removed, along with the aliases it no longer needs.
//! The wrapper and decoder declarations are left to dead-code elimination.
//! In a classic script those are globals other scripts can see, so the
//! rotation IIFE and the top-level aliases stay; only the calls are inlined.
//!
//! The analysis bails on anything it cannot prove: a direct `eval`, a
//! reassigned or redeclared decoder, wrapper or alias (including a block
//! function of the same name, which Annex B may hoist), any other use of the
//! wrapper, code that could run before the rotation, in a script a global
//! `eval`/`Function` or a write through the global object, a checksum it cannot
//! evaluate exactly or that never matches, calls inside `with`, and indexes
//! out of range. It assumes the standard `parseInt`, `Array.prototype.push`
//! and `shift`, and that nothing outside the program writes the decoder
//! through the global object.
//!
//! `stringArrayEncoding` (`base64`, `rc4`) and `stringArrayWrappersType:
//! 'function'` produce other decoder shapes, which are left unchanged.

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod analysis;
mod checksum;
mod pass;
mod shape;

pub use pass::SelfDefendingStringArray;

#[cfg(test)]
mod tests;
