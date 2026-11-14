# AST-First Architecture Plan

## Current Problem

**Token-based pipeline:**
```
Original JS → Tokenize → 20 token passes → join(" ") → Broken syntax → Oxc fails
```

**Key finding:** Oxc CAN parse ALL original bundles perfectly! The problem is our token-based transformations break syntax.

## New Architecture

**AST-based pipeline:**
```
Original JS → Parse with Oxc → AST transformations → Codegen with Oxc → Valid JS
```

## Benefits

1. **Guaranteed valid output** - AST transformations preserve syntax
2. **Full Oxc optimizations** - Can use entire oxc_minifier suite
3. **Single parse/codegen** - Better performance
4. **More maintainable** - Follow oxc_minifier patterns
5. **Composable** - All passes use `Traverse<'a, DeobfuscateState>`

## Conversion Priority

### Phase 1: Core Deobfuscation (High Priority)
These are the most important passes that handle obfuscator-specific patterns:

1. **object_dispatcher** - Inline dispatcher switch statements (CRITICAL)
2. **rotation** - Deobfuscate rotation patterns
3. **string_array** - Inline string arrays
4. **decoder** - Inline decoder functions
5. **control_flow** - Unflatten control flow

### Phase 2: Semantic Optimizations (Medium Priority)
These improve code quality:

6. **constant_folding** - Fold constant expressions
7. **expression_simplify** - Simplify boolean/arithmetic expressions
8. **algebraic_simplify** - Algebraic identities
10. **dead_code** - Remove unreachable code
11. **dead_var_elimination** - Remove unused variables

### Phase 3: Code Cleanup (Lower Priority)
These are mostly syntax normalizations:

12. **function_inline** - Inline single-use functions
13. **call_proxy** - Inline call proxy patterns
14. **operator_proxy** - Inline operator proxies
15. **array_unpack** - Unpack array accesses
16. **dynamic_property** - Convert computed to static properties
17. **try_catch** - Remove empty try-catch
18. **ternary** - Simplify ternary chains
19. **object_sparsing** - Consolidate sparse objects
21. **boolean_literals** - Replace !0/!1
22. **void_replacer** - Replace void 0

### Already Implemented (AST-based)
- ✅ loop_unroll - Unroll constant loops
- ✅ cse - Common subexpression elimination

## Implementation Pattern

Each pass follows the oxc_minifier pattern:

```rust
use oxc_traverse::{Traverse, TraverseCtx};

pub struct DispatcherInliner {
    changed: bool,
    dispatchers: Vec<DispatcherInfo>,
}

impl<'a> Traverse<'a, DeobfuscateState> for DispatcherInliner {
    fn enter_function(&mut self, func: &mut Function<'a>, ctx: &mut Ctx<'a>) {
        // Find and inline dispatcher patterns
    }
}
```

## State Management

```rust
pub struct DeobfuscateState {
    pub changed: bool,
    pub string_arrays: Vec<StringArrayInfo>,
    pub decoders: Vec<DecoderInfo>,
    // ... other analysis results
}
```

## Main Deobfuscator

```rust
pub struct AstDeobfuscator {
    // Analysis passes
    string_array_detector: StringArrayDetector,
    decoder_detector: DecoderDetector,
    
    // Transformation passes
    dispatcher_inliner: DispatcherInliner,
    rotation_deobfuscator: RotationDeobfuscator,
    // ... all other passes
}

impl AstDeobfuscator {
    pub fn deobfuscate(&mut self, code: &str) -> Result<String> {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let parse_result = Parser::new(&allocator, code, source_type).parse();
        
        if !parse_result.errors.is_empty() {
            return Err("Parse failed");
        }
        
        let mut program = parse_result.program;
        
        // Phase 1: Analysis
        let state = self.analyze(&program, &allocator)?;
        
        // Phase 2: Transformations
        let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
        let mut ctx = ReusableTraverseCtx::new(state, scoping, &allocator);
        
        // Run all transformation passes
        traverse_mut_with_ctx(&mut self.dispatcher_inliner, &mut program, &mut ctx);
        traverse_mut_with_ctx(&mut self.rotation_deobfuscator, &mut program, &mut ctx);
        // ... all other passes
        
        // Phase 3: Codegen
        let output = Codegen::new().build(&program).code;
        Ok(output)
    }
}
```

## Migration Strategy

1. Create new `ast_deobfuscate` module alongside existing `deobfuscate`
2. Implement passes one by one, testing against real bundles
3. Once complete, make AST version the default
4. Keep token-based as legacy fallback option

## Testing

Each pass should have:
- Unit tests with simple obfuscated patterns
- Integration tests with real bundle snippets
- Regression tests comparing token vs AST output

## Success Criteria

- ✅ All original bundles parse successfully
- ✅ All transformations preserve valid JavaScript
- ✅ Output is semantically equivalent to input
- ✅ Deobfuscation quality matches or exceeds token-based version
- ✅ Performance is acceptable (<10s for 5MB bundle)
