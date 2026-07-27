//! Command-line arguments and `obsidian://` URIs.
//!
//! Obsidian does have an official CLI (<https://obsidian.md/cli>), but it
//! drives the *running desktop app* — it can't do anything with Obsidian
//! closed. obsidian-tui is the other way round: it reads and writes the vault's
//! Markdown directly. It accepts ordinary flags plus the same `obsidian://`
//! URIs the desktop app registers, so a link or script that opens Obsidian also
//! opens this.

use std::path::PathBuf;

/// What the user asked for on startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    /// Vault directory. Falls back to config, then to Obsidian's own list.
    pub vault: Option<PathBuf>,
    /// Note to open once the vault is loaded.
    pub note: Option<String>,
    /// Query to open the search overlay with.
    pub search: Option<String>,
    /// Open today's daily note.
    pub daily: bool,
    /// Start in the graph view.
    pub graph: bool,
    /// Theme override for this run.
    pub theme: Option<String>,
    /// Send one message to the assistant and print the reply, without the TUI.
    pub prompt: Option<String>,
    pub command: Command,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Command {
    #[default]
    Run,
    ListVaults,
    Help,
    Version,
}

/// Parsing failed; the message is meant for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

/// Parses arguments, excluding the program name.
pub fn parse(args: &[String]) -> Result<Args, ParseError> {
    let mut parsed = Args::default();
    let mut iter = args.iter().peekable();

    while let Some(arg) = iter.next() {
        let mut value = |name: &str| -> Result<String, ParseError> {
            iter.next()
                .cloned()
                .ok_or_else(|| ParseError(format!("{name} needs a value")))
        };

        match arg.as_str() {
            "-h" | "--help" => parsed.command = Command::Help,
            "-V" | "--version" => parsed.command = Command::Version,
            "--list-vaults" => parsed.command = Command::ListVaults,
            "-n" | "--note" => parsed.note = Some(value("--note")?),
            "-s" | "--search" => parsed.search = Some(value("--search")?),
            "-t" | "--theme" => parsed.theme = Some(value("--theme")?),
            "-p" | "--prompt" => parsed.prompt = Some(value("--prompt")?),
            "-d" | "--daily" => parsed.daily = true,
            "-g" | "--graph" => parsed.graph = true,
            "--uri" => {
                let uri = value("--uri")?;
                apply_uri(&uri, &mut parsed)?;
            }
            other if other.starts_with("obsidian://") => apply_uri(other, &mut parsed)?,
            other if other.starts_with('-') => {
                return Err(ParseError(format!("unknown option `{other}`")));
            }
            path => {
                if parsed.vault.is_some() {
                    return Err(ParseError(format!("unexpected argument `{path}`")));
                }
                parsed.vault = Some(PathBuf::from(path));
            }
        }
    }

    Ok(parsed)
}

/// Applies an `obsidian://` URI to the parsed arguments.
///
/// Supports the actions that make sense without a running desktop app: `open`,
/// `new` and `search`. The vault is taken by name or path, matching Obsidian.
fn apply_uri(uri: &str, args: &mut Args) -> Result<(), ParseError> {
    let rest = uri
        .strip_prefix("obsidian://")
        .ok_or_else(|| ParseError(format!("not an obsidian URI: {uri}")))?;

    let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut params: Vec<(String, String)> = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        params.push((key.to_string(), percent_decode(value)));
    }
    let get = |key: &str| -> Option<String> {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    };

    // A `path` parameter names a directory; `vault` names one Obsidian knows.
    if let Some(path) = get("path") {
        args.vault = Some(PathBuf::from(path));
    } else if let Some(name) = get("vault") {
        let found = otui_core::vault::discover()
            .into_iter()
            .find(|v| v.name == name || v.path.to_string_lossy() == name);
        match found {
            Some(vault) => args.vault = Some(vault.path),
            None => {
                return Err(ParseError(format!(
                    "no vault named `{name}` is registered with Obsidian"
                )))
            }
        }
    }

    match action {
        "open" => args.note = get("file").or_else(|| get("filepath")),
        "new" => args.note = get("name").or_else(|| get("file")),
        "search" => args.search = get("query"),
        "daily" => args.daily = true,
        other => {
            return Err(ParseError(format!(
                "unsupported obsidian:// action `{other}`"
            )))
        }
    }

    Ok(())
}

/// Decodes `%XX` escapes and `+` as space.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    // A stray `%` is literal rather than an error; URIs from
                    // shells are frequently half-escaped.
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

pub const HELP: &str = "\
obsidian-tui — an Obsidian-like terminal UI for your vault

USAGE:
    obsidian-tui [VAULT] [OPTIONS]
    obsidian-tui 'obsidian://open?vault=Notes&file=Ideas'

ARGS:
    VAULT                  Vault folder. Defaults to the config `vault`, then to
                           the vault Obsidian last had open.

OPTIONS:
    -n, --note <NAME>      Open a note by name or path
    -s, --search <QUERY>   Start with the search overlay open
    -d, --daily            Open today's daily note
    -g, --graph            Start in the graph view
    -t, --theme <NAME>     Use a theme for this run
    -p, --prompt <TEXT>    Ask the assistant one question and print the reply
        --uri <URI>        Handle an obsidian:// URI
        --list-vaults      List the vaults Obsidian knows about
    -h, --help             Show this help
    -V, --version          Show the version

KEYS (inside the app, press ? for the full list):
    ?       keyboard shortcuts  q       quit (asks first)
    Ctrl+O  quick switcher      Ctrl+P  command palette
    Ctrl+E  read / edit         Ctrl+G  graph view
    Ctrl+L  assistant panel     Ctrl+Q  quit while editing
";

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Args, ParseError> {
        parse(&args.iter().map(|s| (*s).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn no_arguments_runs_with_defaults() {
        let args = parse_args(&[]).expect("parse");
        assert_eq!(args.command, Command::Run);
        assert!(args.vault.is_none());
    }

    #[test]
    fn a_bare_path_is_the_vault() {
        let args = parse_args(&["/notes/vault"]).expect("parse");
        assert_eq!(args.vault, Some(PathBuf::from("/notes/vault")));
    }

    #[test]
    fn flags_parse_with_short_and_long_names() {
        let args = parse_args(&["/v", "-n", "Ideas", "--graph", "--theme", "nord"]).expect("parse");
        assert_eq!(args.note.as_deref(), Some("Ideas"));
        assert!(args.graph);
        assert_eq!(args.theme.as_deref(), Some("nord"));
    }

    #[test]
    fn a_flag_missing_its_value_is_an_error() {
        let err = parse_args(&["--note"]).expect_err("should fail");
        assert!(err.0.contains("--note needs a value"));
    }

    #[test]
    fn unknown_options_are_rejected() {
        assert!(parse_args(&["--nope"]).is_err());
    }

    #[test]
    fn two_paths_are_rejected() {
        assert!(parse_args(&["/a", "/b"]).is_err());
    }

    #[test]
    fn obsidian_open_uri_sets_the_note() {
        let args = parse_args(&["obsidian://open?path=/tmp/vault&file=My%20Note"]).expect("parse");
        assert_eq!(args.vault, Some(PathBuf::from("/tmp/vault")));
        assert_eq!(args.note.as_deref(), Some("My Note"));
    }

    #[test]
    fn obsidian_search_uri_sets_the_query() {
        let args = parse_args(&["--uri", "obsidian://search?query=project+alpha"]).expect("parse");
        assert_eq!(args.search.as_deref(), Some("project alpha"));
    }

    #[test]
    fn an_unsupported_uri_action_is_reported() {
        let err = parse_args(&["obsidian://hook-get-address"]).expect_err("should fail");
        assert!(err.0.contains("unsupported"));
    }

    #[test]
    fn percent_decoding_handles_escapes_and_plus() {
        assert_eq!(percent_decode("My%20Note"), "My Note");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("100%"), "100%", "a stray % stays literal");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
    }

    #[test]
    fn help_mentions_the_uri_form() {
        assert!(HELP.contains("obsidian://"));
        assert!(HELP.contains("--prompt"));
    }
}
