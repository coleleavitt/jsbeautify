#[derive(Debug, Clone, Copy)]
pub struct AnalysisLimits {
    pub max_source_bytes: usize,
    pub max_arrays: usize,
    pub max_functions: usize,
    pub max_decoders: usize,
    pub max_encoded_literal_bytes: usize,
    pub max_decoded_bytes: usize,
    pub max_modules: usize,
    pub max_imports_exports: usize,
    pub max_operators: usize,
    pub max_graph_nodes: usize,
    pub max_graph_edges: usize,
    pub max_errors: usize,
    pub max_report_bytes: usize,
}

impl Default for AnalysisLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 32 * 1024 * 1024,
            max_arrays: 8_192,
            max_functions: 8_192,
            max_decoders: 64,
            max_encoded_literal_bytes: 16 * 1024 * 1024,
            max_decoded_bytes: 12 * 1024 * 1024,
            max_modules: 16,
            max_imports_exports: 8_192,
            max_operators: 32_768,
            max_graph_nodes: 8_192,
            max_graph_edges: 32_768,
            max_errors: 32,
            max_report_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub(super) struct AnalysisBudget {
    pub(super) limits: AnalysisLimits,
    arrays: usize,
    functions: usize,
    decoders: usize,
    encoded_literal_bytes: usize,
    decoded_bytes: usize,
    modules: usize,
    imports_exports: usize,
    operators: usize,
    graph_nodes: usize,
    graph_edges: usize,
    errors: usize,
    exhausted: bool,
}

impl AnalysisBudget {
    pub(super) const fn new(limits: AnalysisLimits) -> Self {
        Self {
            limits,
            arrays: 0,
            functions: 0,
            decoders: 0,
            encoded_literal_bytes: 0,
            decoded_bytes: 0,
            modules: 0,
            imports_exports: 0,
            operators: 0,
            graph_nodes: 0,
            graph_edges: 0,
            errors: 0,
            exhausted: false,
        }
    }

    pub(super) const fn exhausted(&self) -> bool {
        self.exhausted
    }

    pub(super) const fn retain_array(&mut self) -> bool {
        charge_one(&mut self.arrays, self.limits.max_arrays, &mut self.exhausted)
    }

    pub(super) const fn retain_function(&mut self) -> bool {
        charge_one(&mut self.functions, self.limits.max_functions, &mut self.exhausted)
    }

    pub(super) const fn retain_decoder(&mut self) -> bool {
        charge_one(&mut self.decoders, self.limits.max_decoders, &mut self.exhausted)
    }

    pub(super) const fn retain_encoded_literal(&mut self, bytes: usize) -> bool {
        charge(
            &mut self.encoded_literal_bytes,
            bytes,
            self.limits.max_encoded_literal_bytes,
            &mut self.exhausted,
        )
    }

    pub(super) const fn retain_decoded_bytes(&mut self, bytes: usize) -> bool {
        charge(
            &mut self.decoded_bytes,
            bytes,
            self.limits.max_decoded_bytes,
            &mut self.exhausted,
        )
    }

    pub(super) const fn retain_module(&mut self) -> bool {
        charge_one(&mut self.modules, self.limits.max_modules, &mut self.exhausted)
    }

    pub(super) const fn retain_import_export(&mut self) -> bool {
        charge_one(
            &mut self.imports_exports,
            self.limits.max_imports_exports,
            &mut self.exhausted,
        )
    }

    pub(super) const fn visit_operator(&mut self) -> bool {
        charge_one(&mut self.operators, self.limits.max_operators, &mut self.exhausted)
    }

    pub(super) const fn retain_graph_node(&mut self) -> bool {
        charge_one(&mut self.graph_nodes, self.limits.max_graph_nodes, &mut self.exhausted)
    }

    pub(super) const fn retain_graph_edge(&mut self) -> bool {
        charge_one(&mut self.graph_edges, self.limits.max_graph_edges, &mut self.exhausted)
    }

    pub(super) const fn retain_error(&mut self) -> bool {
        charge_one(&mut self.errors, self.limits.max_errors, &mut self.exhausted)
    }

    pub(super) const fn exhaust(&mut self) {
        self.exhausted = true;
    }
}

const fn charge_one(current: &mut usize, limit: usize, exhausted: &mut bool) -> bool {
    charge(current, 1, limit, exhausted)
}

const fn charge(current: &mut usize, amount: usize, limit: usize, exhausted: &mut bool) -> bool {
    if *exhausted || amount > limit.saturating_sub(*current) {
        *exhausted = true;
        return false;
    }
    *current += amount;
    true
}
