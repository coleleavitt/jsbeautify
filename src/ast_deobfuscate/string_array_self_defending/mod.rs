//! String arrays and their decoders: javascript-obfuscator's `stringArray`
//! output, and the same pieces in hand-rolled layouts.
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
//! A decoder may also offset the read itself (`return a[i - OFFSET]`) and
//! pass the element through the global `atob` or through a helper
//! (`return h(a[i], 'key')`) whose body is an XOR or RC4 loop over char
//! codes; helpers are recognised by their body, never by their name (see
//! `decoding`). An array may have several decoders, and code may also read a
//! variable array directly (`a[2]`) once a decoder or a rotation shows it is
//! obfuscated; a plain array of strings is left as written.
//!
//! The pass matches the pieces through oxc symbols, solves the rotation
//! statically, and replaces each `D(<number literal>)` call, made directly
//! or through an alias of `D` (`const a = D`, at any depth), and each `a[N]`
//! read, with the string it returns. It runs before the control-flow
//! unflattener, so that the `'1|0|2'.split('|')` sequences stored in the
//! array are literal there. When no decoder reference or read is left, the
//! rotation IIFE, whose only effect is the array order, is removed, along
//! with the aliases it no longer needs. The array, decoder and helper
//! declarations are left to dead-code elimination. In a classic script those
//! are globals other scripts can see, so the rotation IIFE and the top-level
//! aliases stay; only the calls and reads are inlined.
//!
//! The analysis bails on anything it cannot prove: a direct `eval`, a
//! reassigned or redeclared array, decoder, helper or alias, a binding a
//! sloppy-mode block function may overwrite (Annex B), any use of the array
//! but its decoders, the rotation and property reads outside a call (so no
//! code can change it), a rotation it cannot solve exactly or a second one,
//! code that could run before the rotation, in a script a global
//! `eval`/`Function` or a write through the global object, a local or
//! assigned `atob` or `String`, calls and reads inside `with`, and indexes out
//! of range. It assumes the standard `parseInt`, `atob`, `String.fromCharCode`,
//! `charCodeAt`, `Array.prototype.push` and `shift`, no accessor properties on
//! `Array.prototype` or `Object.prototype`, and that nothing outside the
//! program writes the array, a decoder or a helper through the global object.
//!
//! `stringArrayEncoding` (`base64`, `rc4`) and `stringArrayWrappersType:
//! 'function'` produce other decoder shapes, which are left unchanged.

use oxc_traverse::TraverseCtx;

use crate::ast_deobfuscate::state::DeobfuscateState;

pub type Ctx<'a> = TraverseCtx<'a, DeobfuscateState>;

mod analysis;
mod checksum;
mod decoding;
mod pass;
mod shape;

pub use pass::SelfDefendingStringArray;

#[cfg(test)]
mod tests;
