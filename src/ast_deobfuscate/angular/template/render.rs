use std::fmt::Write as _;

use oxc_ast::ast::{
    Argument, ArrayExpressionElement, BindingPattern, CallExpression, Expression, IdentifierReference,
    StaticMemberExpression, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_semantic::Semantic;
use oxc_span::{GetSpan, Span};
use oxc_syntax::symbol::SymbolId;
use rustc_hash::{FxHashMap, FxHashSet};

use super::extract::{Body, Call, Instructions, TemplateFn, as_function, int_literal};
use super::roles::Role;

const MAX_VIEW_DEPTH: usize = 24;

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr",
];

/// One compiled attribute array from a component's `consts`.
pub(super) type ConstAttrs<'a> = &'a [ArrayExpressionElement<'a>];

/// Everything shared by the views of one bundle.
pub(super) struct Bundle<'s, 'a> {
    pub(super) code: &'s str,
    pub(super) semantic: &'s Semantic<'a>,
    pub(super) roles: &'s [Option<Role>],
    pub(super) views: &'s FxHashMap<u32, Instructions<'a>>,
}

/// Renders one component template to indented HTML-like lines.
pub(super) struct Renderer<'s, 'a> {
    bundle: &'s Bundle<'s, 'a>,
    consts: Vec<ConstAttrs<'a>>,
    /// The component instance parameter of the root template; `ctx.x`
    /// renders as `x`.
    component: Option<SymbolId>,
    active: FxHashSet<u32>,
    pub(super) lines: Vec<String>,
}

enum Kind<'a> {
    Element {
        tag: String,
        void: bool,
    },
    Container,
    Text(String),
    Template {
        body: TemplateFn<'a>,
    },
    Repeater {
        body: TemplateFn<'a>,
        empty: Option<TemplateFn<'a>>,
        track: String,
        collection: String,
    },
    Defer {
        blocks: Vec<(&'static str, u32)>,
    },
    Projection,
}

struct Node<'a> {
    kind: Kind<'a>,
    attrs: Vec<String>,
    bindings: Vec<(String, String)>,
    events: Vec<(String, String)>,
    children: Vec<usize>,
    /// `@if` branches when this template is the first branch of a conditional.
    branches: Option<Vec<(Option<String>, u32)>>,
    consumed: bool,
}

enum Update {
    Text(String),
    Collection(String),
    Branches(Vec<(Option<String>, u32)>),
    Binding(String, String),
}

#[derive(Default)]
struct View<'a> {
    nodes: Vec<Node<'a>>,
    roots: Vec<usize>,
    slots: FxHashMap<u32, usize>,
    pipes: FxHashMap<u32, String>,
}

impl View<'_> {
    fn node_at(&self, slot: u32) -> Option<usize> {
        self.slots.get(&slot).copied()
    }
}

impl<'s, 'a> Renderer<'s, 'a> {
    pub(super) fn new(bundle: &'s Bundle<'s, 'a>, consts: Vec<ConstAttrs<'a>>, root: TemplateFn<'a>) -> Self {
        Self {
            bundle,
            consts,
            component: root.param(1),
            active: FxHashSet::default(),
            lines: Vec::new(),
        }
    }

    pub(super) fn render(&mut self, func: TemplateFn<'a>, depth: usize) {
        if depth > MAX_VIEW_DEPTH || !self.active.insert(func.key()) {
            self.line(depth, "<!-- … -->".to_string());
            return;
        }
        let bundle = self.bundle;
        if let Some(instructions) = bundle.views.get(&func.key()) {
            let view = self.build(instructions);
            for &root in &view.roots {
                self.node(&view, root, depth);
            }
        }
        self.active.remove(&func.key());
    }

    const fn role(&self, call: &Call<'a>) -> Option<Role> {
        self.bundle.roles[call.callee_index]
    }

    fn function(&self, expr: Option<&'a Expression<'a>>) -> Option<TemplateFn<'a>> {
        as_function(expr?, self.bundle.semantic)
    }

    fn build(&self, instructions: &Instructions<'a>) -> View<'a> {
        let mut view = View::default();
        let mut open: Vec<usize> = Vec::new();
        let mut last_element: Option<usize> = None;
        for call in &instructions.create {
            let slot = call.int(0);
            let kind = match (self.role(call), slot) {
                (Some(Role::ElementEnd | Role::ContainerEnd), _) => {
                    open.pop();
                    continue;
                }
                (Some(Role::Pipe), Some(slot)) => {
                    if let Some(name) = call.string(1) {
                        view.pipes.insert(slot, name.to_string());
                    }
                    continue;
                }
                (Some(role @ (Role::ElementStart | Role::Element)), Some(_)) => {
                    let Some(tag) = call.string(1) else { continue };
                    let tag = tag.trim_start_matches(":svg:").trim_start_matches(":math:");
                    Kind::Element {
                        tag: tag.to_string(),
                        void: role == Role::Element,
                    }
                }
                (Some(Role::ContainerStart | Role::Container), Some(_)) => Kind::Container,
                (Some(Role::Text), Some(_)) => Kind::Text(call.string(1).unwrap_or_default().to_string()),
                (Some(Role::Projection), Some(_)) => Kind::Projection,
                (Some(_), _) => continue,
                (None, _) => {
                    if let Some(kind) = self.shaped(call, &view) {
                        kind
                    } else {
                        if let (Some(event), Some(handler), Some(element)) =
                            (call.string(0), self.function(call.arg(1)), last_element)
                        {
                            let handler = self.handler(handler, &view);
                            view.nodes[element].events.push((event.to_string(), handler));
                        }
                        continue;
                    }
                }
            };
            let consts_index = match kind {
                Kind::Element { .. } => call.int(2),
                Kind::Container => call.int(1),
                Kind::Template { .. } | Kind::Repeater { .. } => call.int(5),
                _ => None,
            };
            if let Kind::Defer { blocks } = &kind {
                for &(_, block) in blocks {
                    if let Some(index) = view.node_at(block) {
                        view.nodes[index].consumed = true;
                    }
                }
            }
            let is_element = matches!(kind, Kind::Element { .. });
            let opens = matches!(self.role(call), Some(Role::ElementStart | Role::ContainerStart));
            let index = view.nodes.len();
            view.nodes.push(Node {
                kind,
                attrs: consts_index.map_or_else(Vec::new, |i| self.attrs(i)),
                bindings: Vec::new(),
                events: Vec::new(),
                children: Vec::new(),
                branches: None,
                consumed: false,
            });
            if let Some(slot) = slot {
                view.slots.insert(slot, index);
            }
            match open.last() {
                Some(&parent) => view.nodes[parent].children.push(index),
                None => view.roots.push(index),
            }
            if opens {
                open.push(index);
            }
            if is_element {
                last_element = Some(index);
            }
        }
        self.apply_updates(&mut view, &instructions.update);
        view
    }

    /// Creation calls recognised by their own shape: embedded templates,
    /// `@for` (`repeaterCreate`), and `@defer`.
    fn shaped(&self, call: &Call<'a>, view: &View<'a>) -> Option<Kind<'a>> {
        call.int(0)?;
        if let Some(body) = self.function(call.arg(1)) {
            let track = call.arg(6).filter(|e| {
                matches!(
                    e,
                    Expression::Identifier(_)
                        | Expression::ArrowFunctionExpression(_)
                        | Expression::FunctionExpression(_)
                )
            });
            if let Some(track) = track {
                return Some(Kind::Repeater {
                    body,
                    empty: self.function(call.arg(8)),
                    track: self.expr(track, view),
                    collection: String::new(),
                });
            }
            return Some(Kind::Template { body });
        }
        let primary = call.int(1)?;
        view.node_at(primary)
            .filter(|&i| matches!(view.nodes[i].kind, Kind::Template { .. }))?;
        let mut blocks = vec![("defer", primary)];
        for (label, index) in [("loading", 3), ("placeholder", 4), ("error", 5)] {
            if let Some(slot) = call.int(index)
                && view.node_at(slot).is_some()
            {
                blocks.push((label, slot));
            }
        }
        Some(Kind::Defer { blocks })
    }

    fn apply_updates(&self, view: &mut View<'a>, update: &[Call<'a>]) {
        let mut selected = 0u32;
        for call in update {
            if self.role(call) == Some(Role::Advance) {
                selected = selected.saturating_add(call.int(0).unwrap_or(1));
                continue;
            }
            let explicit = call
                .int(0)
                .filter(|_| call.args.len() == 2)
                .and_then(|slot| view.node_at(slot))
                .filter(|&i| matches!(view.nodes[i].kind, Kind::Repeater { .. } | Kind::Template { .. }));
            let (target, first) = match explicit {
                Some(index) => (index, 1),
                None => match view.node_at(selected) {
                    Some(index) => (index, 0),
                    None => continue,
                },
            };
            let args = &call.args[first..];
            let value = |i: usize| args.get(i).and_then(Argument::as_expression);
            let update = match &view.nodes[target].kind {
                Kind::Text(_) => Update::Text(self.interpolation(args, view)),
                Kind::Repeater { .. } => match value(0) {
                    Some(collection) => Update::Collection(self.expr(collection, view)),
                    None => continue,
                },
                Kind::Template { .. } if !matches!(args.first(), Some(Argument::StringLiteral(_))) => match value(0) {
                    Some(condition) => Update::Branches(self.branches(condition, view)),
                    None => continue,
                },
                _ => match (call.string(first), value(1)) {
                    (Some(name), Some(bound)) => Update::Binding(name.to_string(), self.expr(bound, view)),
                    _ => continue,
                },
            };
            let node = &mut view.nodes[target];
            match update {
                Update::Text(text) => node.kind = Kind::Text(text),
                Update::Collection(text) => {
                    if let Kind::Repeater { collection, .. } = &mut node.kind {
                        *collection = text;
                    }
                }
                Update::Binding(name, bound) => node.bindings.push((name, bound)),
                Update::Branches(branches) => {
                    for &(_, slot) in &branches {
                        if let Some(index) = view.node_at(slot).filter(|&i| i != target) {
                            view.nodes[index].consumed = true;
                        }
                    }
                    view.nodes[target].branches = Some(branches);
                }
            }
        }
    }

    /// `c1 ? 4 : c2 ? 5 : 6` → `[(Some(c1), 4), (Some(c2), 5), (None, 6)]`;
    /// `-1` means no branch.
    fn branches(&self, condition: &'a Expression<'a>, view: &View<'a>) -> Vec<(Option<String>, u32)> {
        let mut out = Vec::new();
        let mut expr = condition;
        loop {
            match expr.without_parentheses() {
                Expression::ConditionalExpression(cond) => {
                    if let Some(slot) = int_literal(&cond.consequent) {
                        out.push((Some(self.expr(&cond.test, view)), slot));
                    }
                    expr = &cond.alternate;
                }
                other => {
                    if let Some(slot) = int_literal(other) {
                        out.push((None, slot));
                    }
                    return out;
                }
            }
        }
    }

    /// `textInterpolate(v)` or `textInterpolateN("a", v1, "b", v2, ...)`.
    fn interpolation(&self, args: &'a [Argument<'a>], view: &View<'a>) -> String {
        let mut text = String::new();
        let parts: Vec<&Expression<'a>> = args.iter().filter_map(Argument::as_expression).collect();
        if let [value] = parts.as_slice() {
            let _ = write!(text, "{{{{ {} }}}}", self.expr(value, view));
            return text;
        }
        for (i, part) in parts.iter().enumerate() {
            match part {
                Expression::StringLiteral(s) if i % 2 == 0 => text.push_str(&s.value),
                value => {
                    let _ = write!(text, "{{{{ {} }}}}", self.expr(value, view));
                }
            }
        }
        text
    }

    /// The handler's returned expression, with its parameter shown as `$event`.
    fn handler(&self, handler: TemplateFn<'a>, view: &View<'a>) -> String {
        handler
            .returned()
            .map_or_else(|| "…".to_string(), |e| self.expr_with(e, view, handler.param(0)))
    }

    fn attrs(&self, index: u32) -> Vec<String> {
        let Some(items) = self.consts.get(index as usize) else {
            return Vec::new();
        };
        let mut attrs = Vec::new();
        let mut classes: Vec<&str> = Vec::new();
        let mut styles: Vec<String> = Vec::new();
        let mut marker: Option<u32> = None;
        let mut pending: Option<&str> = None;
        for item in *items {
            let Some(expr) = item.as_expression() else { continue };
            if let Some(m) = int_literal(expr) {
                marker = Some(m);
                pending = None;
                continue;
            }
            let Expression::StringLiteral(s) = expr else { continue };
            let s = s.value.as_str();
            match marker {
                None => match pending.take() {
                    Some(name) => attrs.push(attr(name, s)),
                    None => pending = Some(s),
                },
                Some(1) => classes.push(s),
                Some(2) => match pending.take() {
                    Some(name) => styles.push(format!("{name}: {s}")),
                    None => pending = Some(s),
                },
                _ => {}
            }
        }
        if !classes.is_empty() {
            attrs.push(attr("class", &classes.join(" ")));
        }
        if !styles.is_empty() {
            attrs.push(attr("style", &styles.join("; ")));
        }
        attrs
    }

    fn node(&mut self, view: &View<'a>, index: usize, depth: usize) {
        let node = &view.nodes[index];
        if node.consumed {
            return;
        }
        match &node.kind {
            Kind::Element { tag, void } => {
                let open = format!("<{tag}{}", attributes(node));
                let only_text = match node.children.as_slice() {
                    [child] => match &view.nodes[*child].kind {
                        Kind::Text(text) => Some(text.trim()),
                        _ => None,
                    },
                    _ => None,
                };
                if *void || (node.children.is_empty() && VOID_ELEMENTS.contains(&tag.as_str())) {
                    self.line(depth, format!("{open} />"));
                } else if node.children.is_empty() {
                    self.line(depth, format!("{open}></{tag}>"));
                } else if let Some(text) = only_text {
                    self.line(depth, format!("{open}>{text}</{tag}>"));
                } else {
                    self.line(depth, format!("{open}>"));
                    self.children(view, node, depth);
                    self.line(depth, format!("</{tag}>"));
                }
            }
            Kind::Container => {
                self.line(depth, format!("<ng-container{}>", attributes(node)));
                self.children(view, node, depth);
                self.line(depth, "</ng-container>".to_string());
            }
            Kind::Text(text) => {
                let text = text.trim();
                if !text.is_empty() {
                    self.line(depth, text.to_string());
                }
            }
            Kind::Projection => self.line(depth, "<ng-content />".to_string()),
            Kind::Template { body } => {
                if let Some(branches) = &node.branches {
                    self.conditional(view, index, branches, depth);
                } else {
                    self.line(depth, format!("<ng-template{}>", attributes(node)));
                    self.render(*body, depth + 1);
                    self.line(depth, "</ng-template>".to_string());
                }
            }
            Kind::Repeater {
                body,
                empty,
                track,
                collection,
            } => {
                let item = body.param(1).and_then(|ctx| self.implicit_alias(*body, ctx));
                let item = item.unwrap_or_else(|| "item".to_string());
                self.line(depth, format!("@for ({item} of {collection}; track {track}) {{"));
                self.render(*body, depth + 1);
                if let Some(empty) = empty {
                    self.line(depth, "} @empty {".to_string());
                    self.render(*empty, depth + 1);
                }
                self.line(depth, "}".to_string());
            }
            Kind::Defer { blocks } => {
                for (i, &(label, slot)) in blocks.iter().enumerate() {
                    let open = if i == 0 {
                        format!("@{label} {{")
                    } else {
                        format!("}} @{label} {{")
                    };
                    self.line(depth, open);
                    if let Some(Kind::Template { body }) = view.node_at(slot).map(|n| &view.nodes[n].kind) {
                        self.render(*body, depth + 1);
                    }
                }
                self.line(depth, "}".to_string());
            }
        }
    }

    fn children(&mut self, view: &View<'a>, node: &Node<'a>, depth: usize) {
        for &child in &node.children {
            self.node(view, child, depth + 1);
        }
    }

    fn conditional(&mut self, view: &View<'a>, first: usize, branches: &[(Option<String>, u32)], depth: usize) {
        let mut rendered = 0;
        for (condition, slot) in branches {
            let Some(index) = view.node_at(*slot) else { continue };
            let Kind::Template { body } = &view.nodes[index].kind else {
                continue;
            };
            let open = match (rendered, condition) {
                (0, Some(c)) => format!("@if ({c}) {{"),
                (0, None) => "@if (…) {".to_string(),
                (_, Some(c)) => format!("}} @else if ({c}) {{"),
                (_, None) => "} @else {".to_string(),
            };
            self.line(depth, open);
            self.render(*body, depth + 1);
            rendered += 1;
        }
        if rendered == 0
            && let Kind::Template { body } = &view.nodes[first].kind
        {
            self.line(depth, "@if (…) {".to_string());
            self.render(*body, depth + 1);
        }
        self.line(depth, "}".to_string());
    }

    /// `let t = ctx.$implicit` inside a `@for` body names the loop item.
    fn implicit_alias(&self, body: TemplateFn<'a>, ctx: SymbolId) -> Option<String> {
        let scoping = self.bundle.semantic.scoping();
        let mut finder = ImplicitAlias {
            scoping,
            ctx,
            found: None,
        };
        match body.body {
            Body::Block(block) => finder.visit_function_body(block),
            Body::Expression(expr) => finder.visit_expression(expr),
        }
        finder.found
    }

    fn line(&mut self, depth: usize, mut text: String) {
        text.insert_str(0, &"  ".repeat(depth));
        self.lines.push(text);
    }

    /// Source text of `expr` with the component prefix dropped and
    /// `pipeBindN(slot, offset, value, ...args)` shown as `value | pipe: args`.
    fn expr(&self, expr: &Expression<'a>, view: &View<'a>) -> String {
        self.expr_with(expr, view, None)
    }

    fn expr_with(&self, expr: &Expression<'a>, view: &View<'a>, event: Option<SymbolId>) -> String {
        let mut edits = ExprEdits {
            renderer: self,
            view,
            event,
            edits: Vec::new(),
        };
        edits.visit_expression(expr);
        let span = expr.span();
        let mut out = String::new();
        let mut cursor = span.start;
        let mut edits = edits.edits;
        edits.sort_by_key(|(s, _)| s.start);
        for (edit, replacement) in edits {
            if edit.start < cursor {
                continue;
            }
            out.push_str(self.slice(Span::new(cursor, edit.start)));
            out.push_str(&replacement);
            cursor = edit.end;
        }
        out.push_str(self.slice(Span::new(cursor, span.end)));
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn slice(&self, span: Span) -> &'s str {
        self.bundle
            .code
            .get(span.start as usize..span.end as usize)
            .unwrap_or_default()
    }
}

fn attr(name: &str, value: &str) -> String {
    if value.is_empty() {
        name.to_string()
    } else {
        format!("{name}=\"{}\"", value.replace('"', "&quot;"))
    }
}

/// Static attributes, then bindings and events; a `[x]` binding paired with
/// an `(xChange)` event collapses to `[(x)]`.
fn attributes(node: &Node<'_>) -> String {
    let mut out = String::new();
    for a in &node.attrs {
        out.push(' ');
        out.push_str(a);
    }
    let two_way = |name: &str| {
        node.events
            .iter()
            .any(|(event, _)| event.strip_suffix("Change") == Some(name))
    };
    for (name, value) in &node.bindings {
        let value = value.replace('"', "&quot;");
        if two_way(name) {
            let _ = write!(out, " [({name})]=\"{value}\"");
        } else {
            let _ = write!(out, " [{name}]=\"{value}\"");
        }
    }
    for (event, handler) in &node.events {
        let bound = event
            .strip_suffix("Change")
            .is_some_and(|name| node.bindings.iter().any(|(b, _)| b == name));
        if !bound {
            let _ = write!(out, " ({event})=\"{}\"", handler.replace('"', "&quot;"));
        }
    }
    out
}

struct ExprEdits<'r, 's, 'a> {
    renderer: &'r Renderer<'s, 'a>,
    view: &'r View<'a>,
    event: Option<SymbolId>,
    edits: Vec<(Span, String)>,
}

impl ExprEdits<'_, '_, '_> {
    fn resolve(&self, ident: &IdentifierReference<'_>) -> Option<SymbolId> {
        self.renderer
            .bundle
            .semantic
            .scoping()
            .get_reference(ident.reference_id())
            .symbol_id()
    }
}

impl<'a> Visit<'a> for ExprEdits<'_, '_, 'a> {
    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        if self.event.is_some() && self.resolve(ident) == self.event {
            self.edits.push((ident.span, "$event".to_string()));
        }
    }

    fn visit_static_member_expression(&mut self, member: &StaticMemberExpression<'a>) {
        if let Expression::Identifier(object) = &member.object
            && self.renderer.component.is_some()
            && self.resolve(object) == self.renderer.component
        {
            self.edits
                .push((Span::new(member.span.start, member.property.span.start), String::new()));
            return;
        }
        walk::walk_static_member_expression(self, member);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let arg = |i: usize| call.arguments.get(i).and_then(Argument::as_expression);
        if let (Some(slot), Some(_), Some(value)) = (arg(0).and_then(int_literal), arg(1).and_then(int_literal), arg(2))
            && let Some(pipe) = self.view.pipes.get(&slot)
        {
            let mut text = format!("{} | {pipe}", self.renderer.expr_with(value, self.view, self.event));
            for extra in call.arguments.iter().skip(3).filter_map(Argument::as_expression) {
                let _ = write!(text, ": {}", self.renderer.expr_with(extra, self.view, self.event));
            }
            self.edits.push((call.span, text));
            return;
        }
        walk::walk_call_expression(self, call);
    }
}

struct ImplicitAlias<'s> {
    scoping: &'s oxc_semantic::Scoping,
    ctx: SymbolId,
    found: Option<String>,
}

impl<'a> Visit<'a> for ImplicitAlias<'_> {
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if self.found.is_none()
            && let BindingPattern::BindingIdentifier(id) = &decl.id
            && let Some(Expression::StaticMemberExpression(member)) = &decl.init
            && member.property.name == "$implicit"
            && let Expression::Identifier(object) = &member.object
            && self.scoping.get_reference(object.reference_id()).symbol_id() == Some(self.ctx)
        {
            self.found = Some(id.name.to_string());
        }
        walk::walk_variable_declarator(self, decl);
    }
}
