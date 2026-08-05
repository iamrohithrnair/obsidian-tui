# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] — 2026-08-05

The release that puts the parts of a vault a text reader could never show —
pictures and drawings — on the screen, and adds an assistant that can be set up
without leaving the app.

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
- Pictures are resampled with a Lanczos filter rather than nearest-neighbour.
  Nearest-neighbour keeps whichever pixel a sample lands on and discards the
  rest, which deletes most of the strokes that make text in a screenshot or a
  diagram legible. The cost is paid once, on the worker thread.
- `images.enabled` and `images.max_height_percent` in the config. The cap is a
  share of the reading pane rather than a fixed number of rows, so a picture is
  as large as the window allows: a cap small enough for an 80x24 terminal leaves
  a diagram unreadable on a full-screen one. A picture is never scaled up, and
  one wider than the pane is scaled down to fit.
- Startup reports which graphics protocol the terminal is using and how big one
  cell is. A terminal that quietly fell back to half-blocks is otherwise
  indistinguishable from one that drew the picture badly.
- **The reading pane pans sideways.** `←`/`→` (or `h`/`l`) move across content
  wider than the window; `g` returns to the top-left. Tables are laid out at
  their content's width and panned across, instead of being squeezed to fit —
  eight columns divided between a narrow pane left three characters each, which
  is not a narrow table but an unreadable one.
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
- **The assistant works behind a corporate TLS proxy.** Such a proxy re-signs
  traffic with the company's own certificate authority, which is in the machine's
  trust store but not in the root list compiled into the binary, so every request
  failed with "unknown issuer". A CA bundle named by `SSL_CERT_FILE`,
  `CURL_CA_BUNDLE`, `REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`,
  `CARGO_HTTP_CAINFO`, `SSL_CERT_DIR` or `OTUI_CA_BUNDLE` is used instead — the
  same convention curl and everything built on OpenSSL follow, so a laptop already
  set up for that network needs no configuration. A directory of certificates works
  as well as a single file, and a bundle that can't be read leaves the built-in
  roots in place rather than trusting nothing.
- `/status` reports which roots and which proxy are in use, which is what
  distinguishes a missing CA from a missing proxy.
- Requests now share one HTTP client, so a turn with several tool calls reuses its
  connection instead of repeating the TLS handshake.
- **The vault opens with its folders closed, and reopens how you left it.**
  Listing every note at once buries the structure the folders exist to express.
  Which folders you left open is remembered per vault in `state.json`, beside
  the config — the folders left *open* are what's stored, so a folder added
  since the last run starts closed rather than springing open. `OTUI_STATE_FILE`
  moves the file elsewhere.
- An outline button in the ribbon, alongside the assistant's, so the right
  sidebar can be toggled with the mouse.
- The shortcut bar lists closing a tab and toggling each pane, and spills onto
  a second row when they don't fit one instead of silently dropping the last
  few.

### Fixed

- **`Ctrl+]` and `Ctrl+\` work.** Neither sidebar toggle could ever fire: a
  terminal without the Kitty keyboard protocol has no way to say "Ctrl and this
  punctuation key" and sends a single control byte instead, which arrives as
  `Ctrl+5` and `Ctrl+4`. Both forms are accepted now. The file explorer's toggle
  was broken the same way and went unnoticed only because the ribbon has a
  button for it.
- **Graph links are visible.** They borrowed the theme's border tone — `#2f2f2f`
  against a `#1e1e1e` background on obsidian-dark, about 1.3:1 — so the graph
  read as a scatter plot. Borders are drawn to be ignored; links are the point.
- Selecting a node now lights up its links properly. A braille cell holds one
  colour, so drawing every link in a single pass let an ordinary one rub out the
  highlight wherever they crossed — which is the middle of the picture.
- Shortcuts in the command palette are no longer clipped. They are right-aligned
  against the list and the scrollbar is painted down its last column, so
  `Ctrl+Shift+F` rendered as `Ctrl+Shift+`. The `?` overlay had the same
  collision.
- **The local graph is laid out on its own.** It was a filtered view of the whole
  vault: positioned among every other note, framed to their bounds, and drawn
  with edges running off to nodes outside the neighbourhood, so it arrived as a
  clump in the corner of an empty pane. The neighbourhood is now cut into a graph
  of its own before anything is positioned.
- The graph no longer rescales on every tick while its layout settles.
- **The graph layout comes to a stop.** It was left to an energy threshold that
  a real vault never reached: a few hundred interlinked notes oscillate below it
  indefinitely, so the graph drifted for its whole step budget — minutes of
  motion — and collapsed into an illegible smear as it went. Motion now decays
  every step, which bounds how far the layout can still travel and makes it
  settle in about a second, spread out and static.
- **Edges meet their nodes.** Nodes are glyphs drawn over edges the canvas
  painted, and the two resolved a position to a cell by different arithmetic —
  disagreeing by up to a whole cell, worse toward the right and bottom of the
  pane — so links stopped beside a note rather than at it.

### Changed

- **The minimum supported Rust version is now 1.90**, up from 1.88. Drawing
  pictures means depending on `ratatui-image`, which reaches `quantette` for
  sixel quantization, and that requires 1.90 — the floor is set by the
  dependency chain rather than by anything in this code.
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

[0.2.0]: https://github.com/iamrohithrnair/obsidian-tui/releases/tag/v0.2.0
[0.1.2]: https://github.com/iamrohithrnair/obsidian-tui/releases/tag/v0.1.2
[0.1.1]: https://github.com/iamrohithrnair/obsidian-tui/releases/tag/v0.1.1
[0.1.0]: https://github.com/iamrohithrnair/obsidian-tui/releases/tag/v0.1.0
