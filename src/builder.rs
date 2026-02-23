//! Builder macro for reducing boilerplate in configuration builders.

/// Generate a builder struct and implementation for a configuration type.
///
/// The configuration type must implement `Default`. The macro generates:
/// - A builder struct with all fields wrapped in `Option`
/// - Setter methods for each field (all accept `impl Into<T>`)
/// - A `build()` method that validates required fields and fills defaults
/// - A `builder()` method on the config type
///
/// # Field categories
///
/// - `required { field: Type }` — `build()` returns an error if not set
/// - `optional { field: Type }` — uses `unwrap_or(defaults.field)` for non-Option fields
/// - `optional_or { field: Type }` — uses `.or(defaults.field)` for `Option<T>` config fields
///
/// Note: For `usize` fields, callers must use suffixed literals (e.g., `10usize`)
/// because `i32 -> usize` has no `Into` impl.
#[allow(unused_macros)]
macro_rules! impl_builder {
    (
        $Config:ident, $Builder:ident {
            required { $( $req_field:ident : $req_ty:ty ),* $(,)? }
            optional { $( $opt_field:ident : $opt_ty:ty ),* $(,)? }
            optional_or { $( $optor_field:ident : $optor_ty:ty ),* $(,)? }
        }
    ) => {
        #[derive(Default)]
        pub struct $Builder {
            $( $req_field: Option<$req_ty>, )*
            $( $opt_field: Option<$opt_ty>, )*
            $( $optor_field: Option<$optor_ty>, )*
        }

        impl $Config {
            pub fn builder() -> $Builder {
                $Builder::default()
            }
        }

        impl $Builder {
            $(
                pub fn $req_field(mut self, value: impl Into<$req_ty>) -> Self {
                    self.$req_field = Some(value.into());
                    self
                }
            )*

            $(
                pub fn $opt_field(mut self, value: impl Into<$opt_ty>) -> Self {
                    self.$opt_field = Some(value.into());
                    self
                }
            )*

            $(
                pub fn $optor_field(mut self, value: impl Into<$optor_ty>) -> Self {
                    self.$optor_field = Some(value.into());
                    self
                }
            )*

            pub fn build(self) -> Result<$Config, $crate::error::BuilderError> {
                let defaults = $Config::default();
                $(
                    let $req_field = self.$req_field.ok_or($crate::error::BuilderError::MissingRequiredField {
                        builder: stringify!($Builder),
                        field: stringify!($req_field),
                    })?;
                )*
                Ok($Config {
                    $( $req_field, )*
                    $( $opt_field: self.$opt_field.unwrap_or(defaults.$opt_field), )*
                    $( $optor_field: self.$optor_field.or(defaults.$optor_field), )*
                })
            }
        }
    };
}

#[allow(unused_imports)]
pub(crate) use impl_builder;
