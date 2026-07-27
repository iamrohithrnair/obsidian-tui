# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.1.0]: https://github.com/iamrohithrnair/obsidian-tui/releases/tag/v0.1.0
