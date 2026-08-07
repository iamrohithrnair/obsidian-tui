//! The built-in theme list, and loading of user themes from disk.

use std::fs;
use std::path::Path;

use crate::seed::{self, Seed};
use crate::theme::Theme;

/// Every built-in theme, in the order the theme picker shows them.
///
/// Obsidian's own two themes come first because they're the point of the app;
/// `terminal` comes last because it's the escape hatch for users who'd rather
/// emeraldian didn't pick colors at all.
pub const ALL: &[Seed] = &[
    seed::OBSIDIAN_DARK,
    seed::OBSIDIAN_LIGHT,
    seed::CATPPUCCIN_MOCHA,
    seed::CATPPUCCIN_MACCHIATO,
    seed::CATPPUCCIN_FRAPPE,
    seed::CATPPUCCIN_LATTE,
    seed::TOKYO_NIGHT,
    seed::TOKYO_NIGHT_STORM,
    seed::GRUVBOX_DARK,
    seed::GRUVBOX_LIGHT,
    seed::NORD,
    seed::SOLARIZED_DARK,
    seed::SOLARIZED_LIGHT,
    seed::DRACULA,
    seed::ROSE_PINE,
    seed::ROSE_PINE_DAWN,
    seed::EVERFOREST_DARK,
    seed::TERMINAL,
];

/// The name of the theme used when config doesn't specify one.
pub const DEFAULT_NAME: &str = "obsidian-dark";

/// Builds every built-in theme.
#[must_use]
pub fn builtin() -> Vec<Theme> {
    ALL.iter().map(build).collect()
}

/// Looks up a built-in theme by name.
#[must_use]
pub fn builtin_by_name(name: &str) -> Option<Theme> {
    ALL.iter().find(|s| s.name == name).map(build)
}

/// The fallback theme, used when the configured one doesn't exist.
#[must_use]
pub fn default_theme() -> Theme {
    build(&seed::OBSIDIAN_DARK)
}

fn build(seed: &Seed) -> Theme {
    let mut theme = Theme::from_seed(seed);

    // `from_seed` draws highlighted text in the page background color, which
    // reads correctly for every theme that has a real background. The terminal
    // theme's background is `reset`, so that would put terminal-default text on
    // a yellow highlight — invisible on most light-on-dark setups.
    if theme.bg_primary == "reset" {
        theme.text_highlight_fg = "black".into();
    }

    theme
}

/// Loads user themes from a directory of TOML files.
///
/// Each file describes one theme; slots it leaves out are inherited from the
/// built-in theme named by its `extends` key (default `obsidian-dark`), so a
/// user theme can be as short as a name and an accent color.
///
/// A missing directory yields no themes rather than an error — not having any
/// custom themes is the normal case. Individual files that fail to parse are
/// skipped and reported, so one bad theme can't stop the app from starting.
pub fn load_custom(dir: &Path) -> (Vec<Theme>, Vec<String>) {
    let mut themes = Vec::new();
    let mut errors = Vec::new();

    let Ok(entries) = fs::read_dir(dir) else {
        return (themes, errors);
    };

    let mut paths: Vec<_> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    // Directory order is filesystem-dependent; sort so the picker is stable.
    paths.sort();

    for path in paths {
        match load_one(&path) {
            Ok(theme) => themes.push(theme),
            Err(err) => errors.push(format!("{}: {err}", path.display())),
        }
    }

    (themes, errors)
}

fn load_one(path: &Path) -> Result<Theme, String> {
    #[derive(serde::Deserialize)]
    struct WithExtends {
        #[serde(default)]
        extends: Option<String>,
    }

    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;

    let extends = toml::from_str::<WithExtends>(&text)
        .ok()
        .and_then(|w| w.extends)
        .unwrap_or_else(|| DEFAULT_NAME.to_string());
    let base = builtin_by_name(&extends).unwrap_or_else(default_theme);

    let mut theme: Theme = toml::from_str(&text).map_err(|e| e.to_string())?;

    // Fall back to the filename so an unnamed theme is still selectable.
    if theme.name.is_empty()
        && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
    {
        theme.name = stem.to_string();
    }

    Ok(theme.layered_over(&base))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Palette;

    #[test]
    fn builtin_names_are_unique() {
        let mut names: Vec<_> = ALL.iter().map(|s| s.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate built-in theme name");
    }

    #[test]
    fn default_theme_is_in_the_list() {
        assert!(builtin_by_name(DEFAULT_NAME).is_some());
    }

    #[test]
    fn every_builtin_resolves() {
        for theme in builtin() {
            let palette = Palette::from(&theme);
            assert_eq!(palette.name, theme.name);
        }
    }

    #[test]
    fn terminal_theme_keeps_highlight_readable() {
        let terminal = builtin_by_name("terminal").expect("terminal theme");
        assert_eq!(terminal.bg_primary, "reset");
        assert_eq!(
            terminal.text_highlight_fg, "black",
            "highlighted text needs an explicit color when the page has none"
        );
    }

    #[test]
    fn custom_theme_inherits_unset_slots() {
        let dir =
            std::env::temp_dir().join(format!("emeraldian-theme-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        fs::write(
            dir.join("mine.toml"),
            "name = \"mine\"\nextends = \"nord\"\naccent = \"#ff0000\"\n",
        )
        .expect("write theme");

        let (themes, errors) = load_custom(&dir);
        fs::remove_dir_all(&dir).ok();

        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(themes.len(), 1);
        let nord = builtin_by_name("nord").expect("nord");
        assert_eq!(themes[0].accent, "#ff0000");
        assert_eq!(themes[0].bg_primary, nord.bg_primary);
    }

    #[test]
    fn missing_custom_dir_is_not_an_error() {
        let (themes, errors) = load_custom(Path::new("/nonexistent/emeraldian/themes"));
        assert!(themes.is_empty());
        assert!(errors.is_empty());
    }
}
