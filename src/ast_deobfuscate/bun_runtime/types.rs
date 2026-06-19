use rustc_hash::FxHashMap;

/// Canonical Bun helper names — used as the rename targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BunHelper {
    Create,
    GetProtoOf,
    DefProp,
    GetOwnPropNames,
    GetOwnPropDesc,
    HasOwnProp,
    AccessProp,
    CopyProps,
    ReExport,
    CommonJs,
    Esm,
    ToEsm,
    ToCommonJs,
    Export,
    Name,
}

impl BunHelper {
    #[must_use]
    pub const fn canonical(&self) -> &'static str {
        match self {
            Self::Create => "__create",
            Self::GetProtoOf => "__getProtoOf",
            Self::DefProp => "__defProp",
            Self::GetOwnPropNames => "__getOwnPropNames",
            Self::GetOwnPropDesc => "__getOwnPropDesc",
            Self::HasOwnProp => "__hasOwnProp",
            Self::AccessProp => "__accessProp",
            Self::CopyProps => "__copyProps",
            Self::ReExport => "__reExport",
            Self::CommonJs => "__commonJS",
            Self::Esm => "__esm",
            Self::ToEsm => "__toESM",
            Self::ToCommonJs => "__toCommonJS",
            Self::Export => "__export",
            Self::Name => "__name",
        }
    }
}

/// Map from mangled name → canonical helper.
pub type BunHelperMap = FxHashMap<String, BunHelper>;
