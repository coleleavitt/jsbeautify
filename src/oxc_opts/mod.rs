pub mod cse;
pub mod loop_unroll;
pub mod state;

pub use cse::CommonSubexpressionElimination;
pub use loop_unroll::LoopUnroller;
pub use state::OptimizationState;

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

pub struct OxcOptimizer {
    loop_unroller: LoopUnroller,
    cse: CommonSubexpressionElimination,
}

impl OxcOptimizer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            loop_unroller: LoopUnroller::new(),
            cse: CommonSubexpressionElimination::new(),
        }
    }

    /// # Errors
    /// Returns an error if the operation fails.
    pub fn optimize(&mut self, code: &str) -> Result<String, String> {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();

        let parse_result = Parser::new(&allocator, code, source_type).parse();

        if !parse_result.diagnostics.is_empty() {
            return Err(format!("Parse errors: {:?}", parse_result.diagnostics));
        }

        let mut program = parse_result.program;

        self.cse.run(&mut program, &allocator);
        self.loop_unroller.run(&mut program, &allocator);

        let output = Codegen::new().build(&program).code;
        Ok(output)
    }
}

impl Default for OxcOptimizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimizer_integration() {
        let code = r"
            for (let i = 0; i < 3; i++) {
                console.log(i);
            }
        ";

        let mut optimizer = OxcOptimizer::new();
        let result = optimizer.optimize(code);
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("console.log(0)"));
        assert!(output.contains("console.log(1)"));
        assert!(output.contains("console.log(2)"));
    }
}
