//! Angular template decompiler — reverses the Angular compiler's output.
//!
//! The Angular compiler transforms `@Component({template: '<div>{{name}}</div>'})` into:
//! ```js
//! ɵɵdefineComponent({
//!   template: function(rf, ctx) {
//!     if (1 & rf) { ɵɵelementStart(0, "div"); ɵɵtext(1); ɵɵelementEnd(); }
//!     if (2 & rf) { ɵɵadvance(); ɵɵtextInterpolate(ctx.name); }
//!   }
//! })
//! ```
//!
//! This module reverses that process, reconstructing approximate HTML templates and
//! emitting annotated pseudo-TypeScript with `@Component`/`@Directive` decorators.
//!
//! ## Instruction → HTML mapping (Angular 10-19):
//! | Instruction                    | HTML equivalent                        |
//! |-------------------------------|----------------------------------------|
//! | `ɵɵelementStart(idx, "div")`  | `<div>`                               |
//! | `ɵɵelementEnd()`              | `</div>`                              |
//! | `ɵɵelement(idx, "br")`        | `<br/>`                               |
//! | `ɵɵtext(idx)`                 | text node (bound)                     |
//! | `ɵɵtext(idx, "literal")`      | literal text node                     |
//! | `ɵɵtemplate(idx, fn, ...)`    | `<ng-template>` / `*ngIf` / `*ngFor`  |
//! | `ɵɵproperty("x", expr)`       | `[x]="expr"`                          |
//! | `ɵɵattribute("x", expr)`      | `[attr.x]="expr"`                     |
//! | `ɵɵlistener("click", fn)`     | `(click)="fn()"`                      |
//! | `ɵɵtextInterpolate(expr)`     | `{{expr}}`                            |
//! | `ɵɵpipe(idx, "name")`         | `| name`                              |
//! | `ɵɵpureFunction*(fn, ...)`    | inline template expression             |
//! | `ɵɵsanitizeHtml`              | innerHTML binding with sanitizer       |
//! | `ɵɵsanitizeUrl`               | URL binding with sanitizer             |
//! | `ɵɵconditional(idx, expr)`    | `@if (expr) { }` (v17+)               |
//! | `ɵɵdefer(idx, ...)`           | `@defer { }` (v17+)                   |
//! | `ɵɵdeclareLet(idx)`           | `@let x = ...` (v19+)                 |
//!
//! ## Consts array mapping:
//! The `consts` array in `ɵɵdefineComponent` contains attribute arrays referenced by index:
//! ```js
//! consts: [["id", "main"], ["class", "header", "role", "banner"]]
//! // index 0: id="main"
//! // index 1: class="header" role="banner"
//! ```
//! These are referenced by `ɵɵelementStart(idx, "tag", constsIdx)`.

mod api;
mod emit;
mod parser;
mod patterns;
mod types;

pub use api::decompile_template_from_source;
pub use emit::emit_pseudo_typescript;
pub use parser::TemplateDecompiler;
pub use types::{DecompiledComponent, TemplateBinding, TemplateNode, ViewEncapsulation};

#[cfg(test)]
mod tests;
