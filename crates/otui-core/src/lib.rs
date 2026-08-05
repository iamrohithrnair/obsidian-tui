//! Core engine for obsidian-tui.
pub mod error;
pub mod excalidraw;
pub mod graph;
pub mod index;
pub mod links;
pub mod markdown;
pub mod note;
pub mod ops;
pub mod search;
pub mod sort;
pub mod vault;

/// Throwaway vaults for tests.
///
/// Behind a feature flag rather than `#[cfg(test)]` so the TUI crate's tests can
/// build real vaults too, without shipping the helper in release builds.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
