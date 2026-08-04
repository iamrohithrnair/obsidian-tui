# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Pictures are drawn in the reading pane.** `![[chart.png]]` and
  `![alt](assets/chart.png)` render as real images in terminals that support
  Kitty's graphics protocol, iTerm2's, or sixel, and as half-block mosaics
  everywhere else. The terminal is asked what it can do at startup, before the
  alternate screen is entered, since that question is answered on stdin.
- Obsidian's `![[chart.png|400]]` width, in pixels, is honoured. Anything else
  after the pipe stays an alias, which is only ever used as alt text.
- Decoding happens off the draw loop, so a large photo doesn't stall scrolling.
  The rows a picture will need are worked out from its header on the first
  frame, so text below it doesn't jump when the picture arrives.
- `images.enabled` and `images.max_rows` in the config. `max_rows` caps how tall
  one picture may be drawn, so a portrait photo doesn't take several screens on
  its own. A picture is never scaled up, and one wider than the pane is scaled
  down to fit.
- A picture that can't be drawn — no support in the terminal, a missing file, a
  URL on the web — leaves its alt text in place rather than a hole.
- **Excalidraw notes open as drawings.** A `.excalidraw.md` note used to show a
  warning banner and a wall of compressed base64, which is the one thing in a
  vault that a text reader could make no sense of at all. The scene is now drawn
  as vectors on a braille canvas: rectangles, ellipses, diamonds, lines, arrows
  with heads, freehand strokes, and the text labels, rotation included. Both the
  plain `json` and the compressed `compressed-json` block the Obsidian plugin
  writes are read, as is a legacy `.excalidraw` JSON file.
- The drawing is scaled to the pane's width and scrolls vertically like the prose
  it replaces, since diagrams are usually far taller than a terminal.
- Excalidraw draws near-black ink on white paper. A stroke that would vanish into
  the theme's background is drawn in the theme's text colour instead; every colour
  the author chose deliberately is kept.
- **The assistant can be set up from inside the app.** `/provider` offers the
  eight backends it knows how to reach — Anthropic, OpenAI, Ollama, LM Studio,
  OpenRouter, Groq, a custom endpoint, or off — and choosing one sets its address
  as well, so nobody has to remember Ollama's port.
- `/model` asks the provider which models it actually has and offers the list.
  Better than a table shipped in the binary: names change monthly, and a local
  server's list depends on what you've pulled. The request runs off the draw loop,
  so the app stays usable while it waits.
- `/key` stores an API key for the current provider, typed into a prompt that
  shows dots. Kept in `auth.json` beside the config, mode `0600` — deliberately
  not in `config.toml`, which people commit to dotfiles repositories. An exported
  variable still wins, so nothing that worked before behaves differently.
- The chat panel's title now names the model that will answer, or says what is
  missing and which command fixes it.

### Fixed

- **The local graph is laid out on its own.** It was a filtered view of the whole
  vault: positioned among every other note, framed to their bounds, and drawn
  with edges running off to nodes outside the neighbourhood, so it arrived as a
  clump in the corner of an empty pane. The neighbourhood is now cut into a graph
  of its own before anything is positioned.
- The graph no longer rescales on every tick while its layout settles.

### Changed

- **Arrow keys walk the graph.** They moved the camera, which left Tab as the
  only way through the picture, and Tab steps by link count so it can jump across
  the vault. Arrows now select the nearest node in the direction pressed; `hjkl`
  still moves the camera.
- Dragging a node with the mouse pins it, which the layout engine always
  supported and nothing called.
- `a` toggles attachments in the graph.
- **Arrow keys walk the slash-command list.** The list appeared on `/` but could
  only be used by typing a name you already knew; `↑`/`↓` now move through it and
  it scrolls to follow. `Esc` abandons a half-typed command before it closes the
  panel.
- `/logout` deletes the stored key rather than only saying the environment still
  has one, and only mentions the environment variable when it is actually set.

## [0.1.2] — 2026-07-27

### Added

- **The file explorer sorts by modification time.** Notes now open with the most
  recently edited at the top, which is usually where you left off. `s` in the
  explorer steps through the six orders (modified, created and file name, each
  both ways), as does `/sort`; `/sort list` shows them with the current one
  marked. Folders stay alphabetical whichever order is chosen, since a folder's
  timestamp changes for reasons that aren't visible on screen.
- The order is stored as `ui.sort_order` in the config, so it survives a
  restart. An unrecognised value falls back to the default rather than costing
  you the rest of the file.
- `NoteMeta` gained a `created` timestamp, falling back to the modification time
  on filesystems that don't record one.
- The `?` overlay gained a "File explorer" section, which it had been missing.

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

[0.1.2]: https://github.com/iamrohithrnair/obsidian-tui/releases/tag/v0.1.2
[0.1.1]: https://github.com/iamrohithrnair/obsidian-tui/releases/tag/v0.1.1
[0.1.0]: https://github.com/iamrohithrnair/obsidian-tui/releases/tag/v0.1.0
