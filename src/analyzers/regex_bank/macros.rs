macro_rules! lazy_regex {
    ($name:ident, $pat:expr) => {
        static $name: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new($pat).expect(concat!("Failed to compile ", stringify!($name)))
        });
    };
}

pub(super) use lazy_regex;
