#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HtmlExtractSources(u16);

impl HtmlExtractSources {
    pub const BODY_TEXT: Self = Self(1 << 0);
    pub const COMMENTS: Self = Self(1 << 1);
    pub const IMG_ALT: Self = Self(1 << 2);
    pub const META_CONTENT: Self = Self(1 << 3);
    pub const TITLE: Self = Self(1 << 4);
    pub const DESCRIPTION: Self = Self(1 << 5);
    pub const TOPIC: Self = Self(1 << 6);
    pub const SUMMARY: Self = Self(1 << 7);
    pub const QUERY_PARAM: Self = Self(1 << 8);

    #[must_use]
    pub const fn all() -> Self {
        Self(
            Self::BODY_TEXT.0
                | Self::COMMENTS.0
                | Self::IMG_ALT.0
                | Self::META_CONTENT.0
                | Self::TITLE.0
                | Self::DESCRIPTION.0
                | Self::TOPIC.0
                | Self::SUMMARY.0
                | Self::QUERY_PARAM.0,
        )
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

impl Default for HtmlExtractSources {
    fn default() -> Self {
        Self::all()
    }
}

#[derive(Debug, Clone, Default)]
pub struct HtmlExtractConfig {
    sources: HtmlExtractSources,
}

impl HtmlExtractConfig {
    #[must_use]
    pub const fn from_sources(sources: HtmlExtractSources) -> Self {
        Self { sources }
    }

    #[must_use]
    pub const fn sources(&self) -> HtmlExtractSources {
        self.sources
    }

    #[must_use]
    pub const fn body_text(&self) -> bool {
        self.sources.contains(HtmlExtractSources::BODY_TEXT)
    }

    #[must_use]
    pub const fn comments(&self) -> bool {
        self.sources.contains(HtmlExtractSources::COMMENTS)
    }

    #[must_use]
    pub const fn img_alt(&self) -> bool {
        self.sources.contains(HtmlExtractSources::IMG_ALT)
    }

    #[must_use]
    pub const fn meta_content(&self) -> bool {
        self.sources.contains(HtmlExtractSources::META_CONTENT)
    }

    #[must_use]
    pub const fn title(&self) -> bool {
        self.sources.contains(HtmlExtractSources::TITLE)
    }

    #[must_use]
    pub const fn description(&self) -> bool {
        self.sources.contains(HtmlExtractSources::DESCRIPTION)
    }

    #[must_use]
    pub const fn topic(&self) -> bool {
        self.sources.contains(HtmlExtractSources::TOPIC)
    }

    #[must_use]
    pub const fn summary(&self) -> bool {
        self.sources.contains(HtmlExtractSources::SUMMARY)
    }

    #[must_use]
    pub const fn query_param(&self) -> bool {
        self.sources.contains(HtmlExtractSources::QUERY_PARAM)
    }

    #[must_use]
    pub const fn with_body_text(mut self, enabled: bool) -> Self {
        self.sources.set(HtmlExtractSources::BODY_TEXT, enabled);
        self
    }

    #[must_use]
    pub const fn with_comments(mut self, enabled: bool) -> Self {
        self.sources.set(HtmlExtractSources::COMMENTS, enabled);
        self
    }

    #[must_use]
    pub const fn with_img_alt(mut self, enabled: bool) -> Self {
        self.sources.set(HtmlExtractSources::IMG_ALT, enabled);
        self
    }

    #[must_use]
    pub const fn with_meta_content(mut self, enabled: bool) -> Self {
        self.sources.set(HtmlExtractSources::META_CONTENT, enabled);
        self
    }

    #[must_use]
    pub const fn with_title(mut self, enabled: bool) -> Self {
        self.sources.set(HtmlExtractSources::TITLE, enabled);
        self
    }

    #[must_use]
    pub const fn with_description(mut self, enabled: bool) -> Self {
        self.sources.set(HtmlExtractSources::DESCRIPTION, enabled);
        self
    }

    #[must_use]
    pub const fn with_topic(mut self, enabled: bool) -> Self {
        self.sources.set(HtmlExtractSources::TOPIC, enabled);
        self
    }

    #[must_use]
    pub const fn with_summary(mut self, enabled: bool) -> Self {
        self.sources.set(HtmlExtractSources::SUMMARY, enabled);
        self
    }

    #[must_use]
    pub const fn with_query_param(mut self, enabled: bool) -> Self {
        self.sources.set(HtmlExtractSources::QUERY_PARAM, enabled);
        self
    }
}
