//! `coterie-provision` — end-to-end install wizard for Coterie.
//!
//! Side-effecting code goes through the `SystemCommand` / `FileSystem`
//! traits in [`system`] / [`fs_ops`]. Production uses `RealSystem` /
//! `RealFs`; tests use the fakes in [`test_support`].

pub mod caddyfile;
pub mod env_template;
pub mod fs_ops;
pub mod github_api;
pub mod install;
pub mod prompts;
pub mod stripe_check;
pub mod system;
pub mod version_selector;

// `test_support` is unconditionally compiled. It's used both by
// integration tests in this crate's `tests/` dir (which build the
// crate as an external dep, so `cfg(test)` gating wouldn't expose it)
// and by the wizard's `--dry-run` plumbing.
pub mod test_support;
