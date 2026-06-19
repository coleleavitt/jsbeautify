use regex::Regex;
use std::sync::LazyLock;

// ─── Regex patterns for instruction parsing ───

// ɵɵelementStart(0, "div") or ɵɵelementStart(0, "div", 3)
pub(super) static ELEMENT_START_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:ɵɵ|\w+\.)elementStart\((\d+),\s*"([^"]+)"(?:,\s*(\d+))?\)"#).unwrap());

// ɵɵelementEnd()
pub(super) static ELEMENT_END_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:ɵɵ|\w+\.)elementEnd\(\)").unwrap());

// ɵɵelement(0, "br") or ɵɵelement(0, "br", 3)
pub(super) static ELEMENT_VOID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:ɵɵ|\w+\.)element\((\d+),\s*"([^"]+)"(?:,\s*(\d+))?\)"#).unwrap());

// ɵɵtext(1) or ɵɵtext(1, "literal")
pub(super) static TEXT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:ɵɵ|\w+\.)text\((\d+)(?:,\s*"([^"]*)")?\)"#).unwrap());

// ɵɵlistener("click", function($event) { ... })
pub(super) static LISTENER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:ɵɵ|\w+\.)listener\("([^"]+)",\s*function\(([^)]*)\)"#).unwrap());

// ɵɵpipe(2, "async")
pub(super) static PIPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:ɵɵ|\w+\.)pipe\((\d+),\s*"([^"]+)"\)"#).unwrap());

// ɵɵadvance(2) or ɵɵadvance()
pub(super) static ADVANCE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:ɵɵ|\w+\.)advance\((\d*)\)").unwrap());

// ɵɵproperty("ngIf", ctx.show)
pub(super) static PROPERTY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:ɵɵ|\w+\.)property\("([^"]+)",\s*(.+?)\)"#).unwrap());

// ɵɵattribute("role", ctx.role)
pub(super) static ATTRIBUTE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:ɵɵ|\w+\.)attribute\("([^"]+)",\s*(.+?)\)"#).unwrap());

// ɵɵtextInterpolate(ctx.name) or ɵɵtextInterpolate1("Hello ", ctx.name, "!")
pub(super) static TEXT_INTERPOLATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:ɵɵ|\w+\.)textInterpolate").unwrap());
