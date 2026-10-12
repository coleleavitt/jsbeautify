macro_rules! counter_pass {
    ($ty:ty, $field:ident, $getter:ident) => {
        impl $ty {
            #[must_use]
            pub const fn new() -> Self {
                Self { $field: 0 }
            }

            #[must_use]
            pub const fn $getter(&self) -> usize {
                self.$field
            }
        }

        impl Default for $ty {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

pub(crate) use counter_pass;
