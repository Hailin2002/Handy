//! Application settings: schema, defaults, persistence, and value types.
//!
//! Split for navigability — `schema.rs` owns the `AppSettings` struct plus its
//! serde default functions (they must live in the same module), `store.rs`
//! handles load/salvage/migrate/write against the plugin-store JSON, and
//! `types.rs` holds the small enums and the redacting `SecretMap`.

pub(crate) mod schema;
mod store;
#[cfg(test)]
mod tests;
mod types;

pub use schema::{get_default_settings, AppSettings};
/// Re-exported as stable public API even though only `store.rs` consumes it today.
#[allow(unused_imports)]
pub use store::SETTINGS_STORE_PATH;
pub use store::{
    get_bindings, get_history_limit, get_recording_retention_period, get_settings,
    get_stored_binding, load_or_create_app_settings, write_settings,
};
pub use types::*;
