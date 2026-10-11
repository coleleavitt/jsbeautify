use oxc_ast::ast::{BindingPattern, Expression, Program, Statement, VariableDeclarator};
use oxc_traverse::Traverse;
use rustc_hash::FxHashMap;

use crate::ast_deobfuscate::state::DeobfuscateState;

use super::{
    AnthropicModification, BundleType, Ctx, EsbuildHelperInfo, EsbuildHelperKind, ObjectAliasKind, SupportHelperKind,
    WeakMapCacheKind,
};

/// Collector for esbuild runtime helpers
pub struct EsbuildHelperCollector {
    pub(super) helpers: FxHashMap<String, EsbuildHelperInfo>,
    pub(super) object_aliases: FxHashMap<String, ObjectAliasKind>,
    pub(super) module_wrappers: Vec<String>,
    pub(super) weakmap_caches: FxHashMap<String, WeakMapCacheKind>,
    pub(super) support_helpers: FxHashMap<String, SupportHelperKind>,
    pub(super) bundle_type: BundleType,
}

impl EsbuildHelperCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            helpers: FxHashMap::default(),
            object_aliases: FxHashMap::default(),
            module_wrappers: Vec::new(),
            weakmap_caches: FxHashMap::default(),
            support_helpers: FxHashMap::default(),
            bundle_type: BundleType::Unknown,
        }
    }

    #[must_use]
    pub const fn get_helpers(&self) -> &FxHashMap<String, EsbuildHelperInfo> {
        &self.helpers
    }

    #[must_use]
    pub const fn get_object_aliases(&self) -> &FxHashMap<String, ObjectAliasKind> {
        &self.object_aliases
    }

    #[must_use]
    pub fn get_module_wrappers(&self) -> &[String] {
        &self.module_wrappers
    }

    #[must_use]
    pub const fn get_weakmap_caches(&self) -> &FxHashMap<String, WeakMapCacheKind> {
        &self.weakmap_caches
    }

    #[must_use]
    pub const fn get_support_helpers(&self) -> &FxHashMap<String, SupportHelperKind> {
        &self.support_helpers
    }

    #[must_use]
    pub const fn get_bundle_type(&self) -> BundleType {
        self.bundle_type
    }

    fn determine_bundle_type(&mut self) {
        let has_anthropic_export = self.helpers.values().any(|h| {
            matches!(h.kind, EsbuildHelperKind::Export)
                && h.anthropic_modifications
                    .iter()
                    .any(|m| matches!(m, AnthropicModification::ConfigurableExport))
        });

        let has_weakmap_cache = !self.weakmap_caches.is_empty();
        let has_support_helpers = !self.support_helpers.is_empty();

        self.bundle_type = if has_anthropic_export || has_weakmap_cache || has_support_helpers {
            BundleType::Anthropic
        } else if !self.helpers.is_empty() {
            BundleType::Standard
        } else {
            BundleType::Unknown
        };
    }
}

impl EsbuildHelperCollector {
    fn try_detect_module_wrapper(&mut self, declarator: &VariableDeclarator<'_>) {
        let Some(init) = &declarator.init else {
            return;
        };

        let var_name = match &declarator.id {
            BindingPattern::BindingIdentifier(ident) => ident.name.as_str().to_string(),
            _ => return,
        };

        let Expression::CallExpression(call) = init else {
            return;
        };

        let callee_name = match &call.callee {
            Expression::Identifier(ident) => ident.name.as_str(),
            _ => return,
        };

        if let Some(helper) = self.helpers.get(callee_name)
            && matches!(helper.kind, EsbuildHelperKind::CommonJs | EsbuildHelperKind::Esm)
        {
            eprintln!(
                "[ESBUILD] Detected module wrapper: {} using {:?}",
                var_name, helper.kind
            );
            self.module_wrappers.push(var_name);
        }
    }
}

impl Default for EsbuildHelperCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Traverse<'a, DeobfuscateState> for EsbuildHelperCollector {
    fn enter_program(&mut self, program: &mut Program<'a>, _ctx: &mut Ctx<'a>) {
        for stmt in &program.body {
            match stmt {
                Statement::VariableDeclaration(var_decl) => {
                    for declarator in &var_decl.declarations {
                        self.try_detect_object_alias(declarator);
                        self.try_detect_prototype_alias(declarator);

                        if let Some(init) = &declarator.init {
                            match init {
                                Expression::ArrowFunctionExpression(arrow) => {
                                    if let BindingPattern::BindingIdentifier(ident) = &declarator.id {
                                        let var_name = ident.name.as_str();
                                        self.try_detect_helper(var_name, arrow, declarator.span.start);
                                    }
                                }
                                Expression::FunctionExpression(func) => {
                                    if let BindingPattern::BindingIdentifier(ident) = &declarator.id {
                                        let var_name = ident.name.as_str();
                                        self.try_detect_function_helper(var_name, func, declarator.span.start);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Statement::FunctionDeclaration(func) => {
                    if let Some(id) = &func.id {
                        let var_name = id.name.as_str();
                        self.try_detect_function_helper(var_name, func, func.span.start);
                    }
                }
                _ => {}
            }
        }

        for stmt in &program.body {
            if let Statement::VariableDeclaration(var_decl) = stmt {
                for declarator in &var_decl.declarations {
                    self.try_detect_module_wrapper(declarator);
                }
            }
        }

        self.determine_bundle_type();

        eprintln!(
            "[ESBUILD] Summary: {} helpers, {} object aliases, {} module wrappers, {} support helpers, bundle type: {:?}",
            self.helpers.len(),
            self.object_aliases.len(),
            self.module_wrappers.len(),
            self.support_helpers.len(),
            self.bundle_type
        );
    }
}
