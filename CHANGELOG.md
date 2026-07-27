# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] — 2026-07-27

### Fixed

- **Graph nodes render as solid discs.** Nodes were drawn by sampling a fixed
  17 points regardless of size, which left gaps as soon as a node was more than
  a couple of dots across and turned the graph into a field of speckle. They are
  now rasterized onto the braille canvas's own dot lattice. Unresolved links and
  attachments draw as hollow rings, so the notes you meant to write stand out.
- **Resetting the graph view frames the graph.** `0` recentred on the origin,
  but the force layout drifts away from it as it settles, so "reset" pushed the
  view off the graph entirely. It now fits to the layout's actual bounds.
- **Labels no longer cover the nodes they name.** Every node's full footprint is
  reserved before any label is placed.
- The graph legend has its own row instead of being painted over the canvas.
- Shortcut hints, the `?` overlay and `--help` listed keys that didn't exist or
  had the wrong case (`l` rather than `L` for labels). They now match the real
  bindings, and tests fail the build if they drift again.
- The README claimed Obsidian has no official CLI. It does.

### Added

- **Slash commands in the assistant panel.** Type `/` for a completion list;
  `Tab` completes, `Enter` runs. Switch backend with `/provider`, `/model` and
  `/base-url`, check credentials with `/login` and `/status`, keep and reload
  conversations with `/save`, `/resume` and `/sessions`, and free up context
  with `/compact`. Commands run locally and never reach the model.
- Conversations are saved as JSON beside the config file, not in the vault.
- **Obsidian CLI integration.** When Obsidian's [official
  CLI](https://obsidian.md/cli) is enabled, `/obsidian` reports its status and
  `/obsidian open` — or "Open this note in Obsidian" in the palette — hands the
  current note to the desktop app. Absent, or with the app closed, obsidian-tui
  says so and carries on.
- Graph keys for fitting (`f`), recentring on the selection (`c`), stepping
  between nodes (`n`/`N`) and rebuilding the layout (`r`).

### Changed

- **`q` quits, and asks first.** It works from the explorer, note pane, sidebar
  and graph, and the prompt names how many notes have unsaved changes. `q` stays
  a letter wherever you might be typing — the editor, the chat box, a search
  field — and `Ctrl+Q` is the way out from there.
- The quit and delete confirmations accept `Enter` as well as `y`; anything else
  still cancels.

## [0.1.0] — 2026-07-27

First release.

### Notes

- Three-pane Obsidian-style layout: icon ribbon, file explorer, note pane with
  tabs, and an outline/backlinks/tags sidebar.
- Live-preview Markdown rendering of Obsidian's dialect — `[[wikilinks]]`
  (dimmed when unresolved), `#tags`, `- [ ]` tasks, `> [!note]` callouts,
  tables, frontmatter, and fenced code with syntax highlighting for Rust,
  Python, JavaScript/TypeScript, Go and shell.
- A text editor with selection, word movement, grouped undo/redo, soft tabs and
  Markdown formatting shortcuts.
- Backlinks with the source line as context, a document outline, and a tag
  browser.
- Following a link to a note that doesn't exist creates it. Renaming a note
  rewrites every wikilink pointing at it. Deletions go to the vault trash.

### Graph

- Force-directed graph view with Barnes-Hut repulsion, so layout is O(n log n)
  and stops consuming CPU once it settles.
- Unresolved links appear as their own nodes — the notes you meant to write.
- Local graph for the open note, filters for tags, orphans and attachments, and
  pan/zoom/select.

### Assistant

- A chat panel backed by a native Rust agent runtime: streaming, tool calling,
  cancellation and usage accounting.
- Fifteen tools over the live vault — search, read, create, append, replace,
  link, rename, delete, list tags, inspect links and neighbourhoods, plus UI
  actions to open a note or focus the graph.
- Tools execute on the UI thread against the same state the user sees, so the
  agent's changes appear immediately.
- Providers: Anthropic Messages API, and any OpenAI-compatible server
  (Ollama, LM Studio, vLLM, OpenRouter) for fully offline use. With no
  credentials the panel explains how to configure one instead of failing.
- `--prompt` answers a single question on stdout without starting the TUI.
- `allow_writes = false` restricts the assistant to reading and searching.

### Interface

- Command palette, quick switcher, global content search, theme picker, vault
  switcher, help overlay, and input/confirmation prompts.
- Context-sensitive shortcut hint bar.
- Mouse support: clickable ribbon buttons, explorer rows, tabs, sidebar panels
  and graph nodes; the scroll wheel targets the pane under the pointer.
- Twenty themes including Obsidian's own light and dark, Catppuccin, Tokyo
  Night, Gruvbox, Nord, Solarized, Dracula, Rosé Pine, Everforest, and a
  `terminal` theme that inherits the terminal's palette. User themes can be
  added as TOML files that inherit from any built-in.

### Vault

- Discovers vaults from Obsidian's own `obsidian.json` on macOS, Linux and
  Windows, and works on any plain folder of Markdown.
- Accepts `obsidian://open`, `new` and `search` URIs alongside ordinary flags.
- Unreadable vaults are reported clearly, including the macOS privacy
  permission that usually causes it.

[0.1.1]: https://github.com/iamrohithrnair/obsidian-tui/releases/tag/v0.1.1
[0.1.0]: https://github.com/iamrohithrnair/obsidian-tui/releases/tag/v0.1.0
