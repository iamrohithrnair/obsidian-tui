# obsidian-tui

An Obsidian-like terminal UI for your vault: the three-pane layout, live-preview
Markdown, backlinks, a force-directed graph, twelve-plus themes, and a built-in
AI assistant that works on your notes through the app's own commands.

It reads a plain folder of Markdown files. Point it at an existing Obsidian
vault and it works — no import, no database, no lock-in. Obsidian can stay open
on the same vault at the same time.

```
 demo-vault │ Welcome.md                                                          obsidian-dark
   ┌ Files ───────────────┐ Welcome                                    ┌ Assistant ─────────────┐
 ≡ │▾ Daily               │  tags: start, meta                         │Ask about your notes.   │
   │▾ Projects            │  ──────────────────────────                │The assistant can       │
 ⌕ │  · Backlog           │                                            │search, read, create    │
   │  · Roadmap           │  Welcome                                   │and link them.          │
 ◈ │· Ideas               │  ───────────────                           │                        │
   │· Orphan              │                                            │                        │
 ✦ │· Welcome             │  This is a demo vault. It links to Ideas,  │                        │
   │                      │  to the roadmap, and to Not Written Yet.   │                        │
 ⚙ │                      │                                            │                        │
   │                      │  ☐ Try the graph view with Ctrl+G          │                        │
   │                      │  ☑ Read this note                          │                        │
   │                      │                                            │                        │
   │                      │  ▎ Callout                                 │                        │
   │                      │  ▎ Callouts render with a colored bar.     │                        │
   │                      │                                            │                        │
   │                      │  ┌─────────┬────────┐                      │                        │
   │                      │  │ Feature │ Status │                      │                        │
   │                      │  ├─────────┼────────┤                      │                        │
   │                      │  │ Reading │   done │                      │                        │
   │                      │  └─────────┴────────┘                      │                        │
   └──────────────────────┘                                            └────────────────────────┘
 READING                                                            84 words  2 backlinks
```

## Install

**Homebrew** (macOS and Linux):

```sh
brew install iamrohithrnair/tap/obsidian-tui
```

**One-line install** (macOS and Linux) — downloads the right build for your
machine and verifies its checksum:

```sh
curl -fsSL https://raw.githubusercontent.com/iamrohithrnair/obsidian-tui/main/install.sh | sh
```

Set `OTUI_BIN_DIR` to choose where it lands, or `OTUI_VERSION` to pin a release.

**npm** — if you'd rather not add another package manager:

```sh
npm install -g obsidian-tui
# or run it without installing:
npx obsidian-tui ~/Notes
```

**Cargo** (needs Rust 1.88 or newer):

```sh
cargo install --git https://github.com/iamrohithrnair/obsidian-tui obsidian-tui
```

**Manual download** — grab an archive from the
[latest release](https://github.com/iamrohithrnair/obsidian-tui/releases/latest):

```sh
tar -xzf obsidian-tui-<target>.tar.gz
shasum -a 256 -c obsidian-tui-<target>.tar.gz.sha256   # optional but cheap
sudo mv obsidian-tui-<target>/obsidian-tui /usr/local/bin/
xattr -d com.apple.quarantine /usr/local/bin/obsidian-tui   # macOS only
```

Prebuilt for `aarch64-apple-darwin` (Apple silicon),
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` and
`x86_64-pc-windows-msvc`. Intel Macs build from source with cargo.

**From a clone:**

```sh
git clone https://github.com/iamrohithrnair/obsidian-tui
cd obsidian-tui
cargo install --path crates/otui --locked   # installs to ~/.cargo/bin
cargo build --release                       # or just build it
```

## Run

```sh
obsidian-tui ~/Notes           # a specific vault
obsidian-tui                   # the vault Obsidian last had open
obsidian-tui --list-vaults     # what Obsidian knows about
```

Obsidian has no official CLI, only its URI scheme — obsidian-tui accepts both:

```sh
obsidian-tui ~/Notes --note "Project Ideas"
obsidian-tui ~/Notes --search "quarterly"
obsidian-tui ~/Notes --daily
obsidian-tui ~/Notes --graph
obsidian-tui 'obsidian://open?vault=Notes&file=Ideas'
```

## Keys

Obsidian's shortcuts where it has them, vim's where it doesn't. `?` shows the
full list in the app.

| | |
|---|---|
| `Ctrl+O` | Quick switcher |
| `Ctrl+P` | Command palette |
| `Ctrl+Shift+F` | Search all notes |
| `Ctrl+E` | Toggle reading / editing |
| `Ctrl+N` / `Ctrl+D` | New note / today's daily note |
| `Ctrl+G` / `Ctrl+Shift+G` | Graph / local graph |
| `Ctrl+L` | Assistant panel |
| `Ctrl+\` / `Ctrl+]` | Toggle the sidebars |
| `Tab` | Move between panes |
| `hjkl`, `g`, `G` | Move within a pane |
| `Enter` | Open / follow a link |
| `Ctrl+Q` | Quit |

A context-sensitive hint bar sits above the status bar showing the keys that
apply where you are; `Ctrl+P` → "Toggle shortcut hints" turns it off.

## Mouse

The ribbon icons are buttons, and most of the UI is clickable:

| | |
|---|---|
| Ribbon icons | Files, search, graph, assistant, palette |
| A note in the explorer | Opens it |
| A folder in the explorer | Folds it |
| A tab | Switches to it |
| Outline / Backlinks / Tags | Switches panel |
| A graph node | Selects it |
| Scroll wheel | Scrolls the pane under the pointer, not the focused one |

## Features

**Notes.** Live-preview Markdown with Obsidian's dialect: `[[wikilinks]]`
(dimmed when they don't resolve yet), `#tags`, `- [ ]` tasks, `> [!note]`
callouts, tables, and fenced code with syntax highlighting. Frontmatter is
optional — a Markdown file dropped in from anywhere shows up.

**Links.** A backlinks pane with the line each link sits on, an outline pane,
and a tag browser. Following a link to a note that doesn't exist creates it, as
Obsidian does. Renaming a note rewrites every wikilink pointing at it.

**Graph.** A force-directed graph with Barnes-Hut repulsion, so it stays
interactive on large vaults and stops burning CPU once it settles. Notes that
are only *linked to* appear as hollow nodes — usually the most useful thing on
the screen, since they're the notes you meant to write. `Ctrl+Shift+G` shows the
neighbourhood of the open note.

**Themes.** Obsidian's own light and dark, plus Catppuccin, Tokyo Night,
Gruvbox, Nord, Solarized, Dracula, Rosé Pine, Everforest, and a `terminal` theme
that inherits your terminal's palette. Drop a TOML file in the themes directory
to add your own; unset colours inherit from whichever theme it `extends`.

**Assistant.** A chat panel that operates on the vault through the same
commands you use. It can search, read, create, edit, rename, link and delete
notes, and open them or the graph on your screen. Every tool call is shown in
the transcript, so you can see what it did rather than trusting a summary.

## The assistant

Set a key and restart:

```sh
export ANTHROPIC_API_KEY=sk-ant-...
```

Or run it entirely offline against a local model:

```toml
[agent]
provider = "openai"                      # any OpenAI-compatible server
base_url = "http://localhost:11434/v1"   # Ollama, LM Studio, vLLM, OpenRouter…
model = "llama3.1"
```

With no key configured the panel still opens and explains how to set one up;
nothing else in the app depends on it.

Scripted use, without the TUI:

```sh
obsidian-tui ~/Notes --prompt "which notes mention the Q3 migration?"
```

Set `allow_writes = false` under `[agent]` to give it search and read only.

## Privacy

**obsidian-tui makes no network connections unless you use the assistant.**
There is no telemetry, no analytics and no update check. Only the `otui-agent`
crate has an HTTP dependency at all; the vault, editor and graph cannot reach
the network.

When you do send a message, what leaves your machine is:

- the message you typed,
- the note you have open, if `include_active_note` is on (it is by default),
- and whatever notes the assistant reads with its tools while answering.

That goes to whichever provider you configured, under that provider's own
terms. Nothing else is transmitted, and nothing is sent in the background.

To keep everything local, point it at a model running on your own machine:

```toml
[agent]
provider = "openai"
base_url = "http://localhost:11434/v1"
model = "llama3.1"
```

Or turn the assistant off entirely with `provider = "offline"`. To keep the
vault out of messages while still using it, set `include_active_note = false`;
to stop it reading notes on its own, set `allow_writes = false` — that leaves
search and read, which still read note contents, so use a local model if that
matters to you.

Your API key is read from the environment (`ANTHROPIC_API_KEY` or
`OPENAI_API_KEY`) and is never written to the config file, never logged, and
never included in any error message.

## Configuration

Written on first run, with every default spelled out:

- macOS: `~/Library/Application Support/obsidian-tui/config.toml`
- Linux: `~/.config/obsidian-tui/config.toml`
- Windows: `%APPDATA%\obsidian-tui\config.toml`

Custom themes go in a `themes/` directory beside it.

## Layout

```
crates/
  otui-core    vault discovery, indexing, markdown, search, graph engine
  otui-theme   the theme model and presets
  otui-agent   provider connectors, streaming, and the tool-calling loop
  otui         the terminal application
```

`otui-core` and `otui-agent` have no dependency on the terminal, and
`otui-agent` has no dependency on the vault — the tools are supplied by the
application, which is what lets the assistant and the user act on exactly the
same state.

## Development

```sh
cargo test --workspace          # 379 tests
cargo clippy --workspace --all-targets
cargo fmt --all --check
```

Releases are cut by tagging. Pushing a `v*` tag builds every target, publishes
a GitHub release with checksums, updates the Homebrew formula in the tap, and
publishes to npm. `CHANGELOG.md` becomes the release notes, so update it first.

Two repository secrets gate the last two steps; without them the release still
succeeds and those jobs just report that they were skipped.

| Secret | Used for |
|---|---|
| `NPM_TOKEN` | Publishing to npm (an automation token) |
| `TAP_GITHUB_TOKEN` | Pushing the formula to `iamrohithrnair/homebrew-tap` |

Packaging lives in `packaging/`. The Homebrew formula is generated by
`packaging/homebrew/update-formula.sh`, which can be run by hand against a
directory of `.sha256` files.

```sh
# bump the version in Cargo.toml, update CHANGELOG.md, then:
git tag -a v0.2.0 -m "v0.2.0"
git push origin v0.2.0
```

## Credits

Built by combining three excellent terminal note-takers: the notebook-style
three-pane layout and theming of [shiki](https://github.com/sazardev/shiki), the
graph and node visualisation of [clin](https://github.com/reekta92/clin-rs), and
the Obsidian vault integration of
[basalt](https://github.com/erikjuhani/basalt). The agent runtime follows the
design of the [pi](https://github.com/earendil-works/pi) agent harness,
reimplemented in Rust.

## License

MIT
