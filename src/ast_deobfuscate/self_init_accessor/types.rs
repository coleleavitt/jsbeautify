#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InitKind {
    EmptyArray,
    EmptyObject,
    NewObject,
    NewArray,
    ObjectCreateNull,
    ObjectCreateProto,
    ArrayIter(&'static str),
}
