#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractSources(u8);

impl ExtractSources {
    pub const VARIABLE_NAMES: Self = Self(1 << 0);
    pub const PROPERTY_NAMES: Self = Self(1 << 1);
    pub const FUNCTION_NAMES: Self = Self(1 << 2);
    pub const CLASS_NAMES: Self = Self(1 << 3);

    #[must_use]
    pub const fn all() -> Self {
        Self(Self::VARIABLE_NAMES.0 | Self::PROPERTY_NAMES.0 | Self::FUNCTION_NAMES.0 | Self::CLASS_NAMES.0)
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn contains(self, source: Self) -> bool {
        self.0 & source.0 == source.0
    }

    pub const fn set(&mut self, source: Self, enabled: bool) {
        if enabled {
            self.0 |= source.0;
        } else {
            self.0 &= !source.0;
        }
    }
}

impl Default for ExtractSources {
    fn default() -> Self {
        Self::all()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExtractConfig {
    sources: ExtractSources,
}

impl ExtractConfig {
    #[must_use]
    pub const fn from_sources(sources: ExtractSources) -> Self {
        Self { sources }
    }

    #[must_use]
    pub const fn sources(&self) -> ExtractSources {
        self.sources
    }

    #[must_use]
    pub const fn variable_names(&self) -> bool {
        self.sources.contains(ExtractSources::VARIABLE_NAMES)
    }

    #[must_use]
    pub const fn property_names(&self) -> bool {
        self.sources.contains(ExtractSources::PROPERTY_NAMES)
    }

    #[must_use]
    pub const fn function_names(&self) -> bool {
        self.sources.contains(ExtractSources::FUNCTION_NAMES)
    }

    #[must_use]
    pub const fn class_names(&self) -> bool {
        self.sources.contains(ExtractSources::CLASS_NAMES)
    }

    #[must_use]
    pub const fn with_variable_names(mut self, enabled: bool) -> Self {
        self.sources.set(ExtractSources::VARIABLE_NAMES, enabled);
        self
    }

    #[must_use]
    pub const fn with_property_names(mut self, enabled: bool) -> Self {
        self.sources.set(ExtractSources::PROPERTY_NAMES, enabled);
        self
    }

    #[must_use]
    pub const fn with_function_names(mut self, enabled: bool) -> Self {
        self.sources.set(ExtractSources::FUNCTION_NAMES, enabled);
        self
    }

    #[must_use]
    pub const fn with_class_names(mut self, enabled: bool) -> Self {
        self.sources.set(ExtractSources::CLASS_NAMES, enabled);
        self
    }
}
