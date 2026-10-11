use super::{InspectError, Inspection, parse};
use oxc_allocator::Allocator;

/// The program as `ESTree` JSON from Oxc's own serializer.
///
/// With `typescript`, the TS-ESTree fields are included. `start`/`end` are
/// UTF-8 byte offsets.
///
/// # Errors
/// Returns [`InspectError::Parse`] when the parser cannot recover.
pub fn estree(source: &str, typescript: bool) -> Result<Inspection<String>, InspectError> {
    let allocator = Allocator::default();
    let (program, diagnostics) = parse(&allocator, source, typescript)?;
    Ok(Inspection {
        report: program.to_pretty_estree_json(typescript, false),
        diagnostics,
    })
}
