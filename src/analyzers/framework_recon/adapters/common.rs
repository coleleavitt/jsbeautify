use crate::analyzers::match_kind::PositionRange;

pub(super) fn pos(source: &str, needle: &str) -> PositionRange {
    source.find(needle).map_or_else(
        || PositionRange::new(1, 0, 1, 0),
        |i| PositionRange::from_byte_range(source, i, i + needle.len()),
    )
}
