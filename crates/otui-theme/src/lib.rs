//! Color themes for obsidian-tui.
//!
//! The flow is: a [`Seed`] (a color scheme's ~20 base colors) expands into a
//! [`Theme`] (~90 semantic slots, serializable, partially overridable by the
//! user), which resolves into a [`Palette`] (the same slots as concrete
//! terminal colors) that the renderer reads every frame.
//!
//! ```
//! use otui_theme::{presets, Palette};
//!
//! let theme = presets::builtin_by_name("obsidian-dark").expect("built-in theme");
//! let palette = Palette::from(&theme);
//! assert_eq!(palette.name, "obsidian-dark");
//! ```

pub mod color;
pub mod presets;
pub mod seed;
pub mod theme;

pub use color::{blend, luminance};
pub use seed::Seed;
pub use theme::{Palette, Theme};

/// A named theme together with its resolved colors.
///
/// The [`Theme`] is kept alongside the [`Palette`] so the theme picker can show
/// and re-save the source colors while the renderer uses the resolved ones.
#[derive(Debug, Clone)]
pub struct ActiveTheme {
    pub theme: Theme,
    pub palette: Palette,
}

impl ActiveTheme {
    #[must_use]
    pub fn new(theme: Theme) -> Self {
        let palette = Palette::from(&theme);
        Self { theme, palette }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.theme.name
    }
}

impl Default for ActiveTheme {
    fn default() -> Self {
        Self::new(presets::default_theme())
    }
}
