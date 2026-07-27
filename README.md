# obsidian-tui

**The best TUI for Obsidian.** Your vault, the way you already know it: the
three-pane layout, live-preview Markdown, backlinks, a force-directed graph,
18 themes. Except it lives in your terminal and never asks you to reach for
the mouse.

Point it at a vault you already have. There's no import step, no database, no
lock-in: it reads the same plain folder of Markdown files Obsidian does, and
you can leave Obsidian open on that folder the whole time. Close this and your
notes are exactly the files they were before.

It also comes with an AI assistant that works on your notes through the very
same commands you do, so you can watch what it did instead of taking its word
for it.

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

The one-liner is the easiest way in. It works out which build fits your machine,
downloads it, and checks it against its published checksum before anything moves:

```sh
curl -fsSL https://obsidian-tui.github.io/install.sh | sh
```

macOS and Linux. Set `OTUI_BIN_DIR` to choose where it lands, or `OTUI_VERSION`
to pin a release. Piping a script into a shell is always worth a look first;
[here it is in full](https://github.com/iamrohithrnair/obsidian-tui/blob/main/install.sh),
and it's a readable 150-odd lines.

Or use whichever package manager you already trust.

**Homebrew** (macOS and Linux):

```sh
brew install iamrohithrnair/tap/obsidian-tui
```

**npm**, if you want to try it before you commit to it:

```sh
npx obsidian-tui ~/Notes     # run it once, install nothing
npm install -g obsidian-tui  # keep it
```

**Cargo** (needs Rust 1.88 or newer):

```sh
cargo install --git https://github.com/iamrohithrnair/obsidian-tui obsidian-tui
```

**Manual download.** Grab an archive from the
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

obsidian-tui accepts ordinary flags and the `obsidian://` URIs the desktop app
registers, so a link or script that opens Obsidian also opens this:

```sh
obsidian-tui ~/Notes --note "Project Ideas"
obsidian-tui ~/Notes --search "quarterly"
obsidian-tui ~/Notes --daily
obsidian-tui ~/Notes --graph
obsidian-tui 'obsidian://open?vault=Notes&file=Ideas'
```

### Alongside Obsidian's own CLI

Obsidian ships an [official CLI](https://obsidian.md/cli), enabled under
Settings → General → "Command line interface". The two do different jobs:

|  | `obsidian` | `obsidian-tui` |
|---|---|---|
| Is | a remote control for the app | the interface itself |
| Talks to | the running desktop app | the vault's files |
| Needs an Obsidian instance | yes, and launches one if none is running | no |
| On a machine with no display | needs `--ozone-platform=headless` or Xvfb | runs as it is |

So they complement each other rather than compete. When the `obsidian` binary is
on your `PATH`, obsidian-tui uses it for the one thing only the app can do,
which is handing a note to the GUI:

- `/obsidian` in the assistant panel reports the CLI's status and the vaults the
  app knows about.
- `/obsidian open`, or "Open this note in Obsidian" in the command palette,
  opens the current note in the desktop app.

If the CLI isn't enabled, or Obsidian isn't running, obsidian-tui says so and
carries on; nothing else depends on it.

## Keys

Obsidian's shortcuts where it has them, the conventions other TUIs use where it
doesn't. `?` shows the full list in the app.

| | |
|---|---|
| `?` | Keyboard shortcuts |
| `q` | Quit (asks first; `Ctrl+Q` works while editing too) |
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

In the file explorer: `/` filters by name, `s` changes the sort order, `Space`
folds a folder, and `H`/`L` collapse or expand every folder at once.

In the graph: `hjkl` pans, `+`/`-` zooms, `f` fits the whole graph on screen,
`Tab`/`Shift+Tab` steps between nodes, `c` recentres on the selection, `L`
toggles labels, `u` unresolved links, `t` tags, and `r` rebuilds the layout.

`q` never quits from somewhere you might be typing: in the editor, the chat box
or a search field it types a `q`, and `Ctrl+Q` is the way out.

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
optional: a Markdown file dropped in from anywhere shows up.

**Explorer.** The file tree opens with your most recently edited notes at the
top, which is usually where you left off. `s` steps through the other orders:
modified, created and file name, each in both directions. Folders stay
alphabetical throughout, and the choice is written to the config file, so it's
still there next time you start.

**Links.** A backlinks pane with the line each link sits on, an outline pane,
and a tag browser. Following a link to a note that doesn't exist creates it, as
Obsidian does. Renaming a note rewrites every wikilink pointing at it.

**Graph.** A force-directed graph with Barnes-Hut repulsion, so it stays
interactive on large vaults and stops burning CPU once it settles. Notes that
are only *linked to* appear as hollow nodes, usually the most useful thing on
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

### Slash commands

Type `/` in the chat box for a completion list; `Tab` completes, `Enter` runs.
Commands are handled locally and never reach the model: `/model` changes the
model rather than asking the current one to.

| | |
|---|---|
| `/help` | List the commands |
| `/new`, `/compact` | Start over, or trim older turns to free up context |
| `/save`, `/resume`, `/sessions` | Keep a conversation and pick it up later |
| `/provider`, `/model`, `/base-url` | Point the agent at a different backend |
| `/login`, `/logout`, `/status` | Credentials and what the next turn will do |
| `/writes`, `/context`, `/reasoning` | Toggle what the agent may do and see |
| `/tools`, `/vault`, `/obsidian` | What's available: tools, index, Obsidian CLI |
| `/sort` | Change how the explorer orders notes (`/sort list` shows them) |
| `/config` | Write the current settings to the config file |
| `/keys`, `/quit` | Shortcut reference, and leave |

Sessions are stored as JSON next to the config file, not in the vault, so a
vault stays a plain folder of Markdown.

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
to stop it reading notes on its own, set `allow_writes = false`. That leaves
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
`otui-agent` has no dependency on the vault: the tools are supplied by the
application, which is what lets the assistant and the user act on exactly the
same state.

## Development

```sh
cargo test --workspace          # 473 tests
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

This project exists because four other people published their work first. None
of their code is in here (obsidian-tui is written from scratch), but every one
of them showed me something I'd otherwise have had to guess at, and the good
ideas are theirs.

**[shiki](https://github.com/sazardev/shiki)** by Omar (MIT). A personal
notebook TUI, and the reason the three-pane layout and the theme model look the
way they do. It's the clearest demonstration I found that a note-taking TUI can
be genuinely nice to look at.

**[clin](https://github.com/reekta92/clin-rs)** (GPL-3.0). An Obsidian-vault
TUI with a graph view. Reading how it handles nodes, edges and viewport
maths taught me most of what I know about drawing a graph in a terminal, and
sent me down the braille-canvas route in the first place.

**[basalt](https://github.com/erikjuhani/basalt)** by Erik Juhani
(Apache-2.0 / GPL-3.0). A TUI for managing Obsidian vaults and notes. The
prior art for treating a vault as nothing more than the folder it already is,
and for how to find the vaults Obsidian knows about.

**[pi](https://github.com/earendil-works/pi)** by Mario Zechner (MIT). A
coding-agent harness. The assistant's architecture follows its shape closely:
the streaming tool-calling loop, and the idea that slash commands are handled
by the client and never reach the model. Reimplemented in Rust; the design
credit is pi's.

Licensing note, since two of these are copyleft: nothing was copied, so the
choice of licence here was a free one rather than an obligation. It went to the
GPL anyway. The debt is one of ideas, and it's a real one.

Not affiliated with Obsidian.md.

## License

Copyright (C) 2026 Rohith Nair.

obsidian-tui is free software: you can redistribute it and/or modify it under
the terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version. See [LICENSE](LICENSE).

It is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR
PURPOSE.

In plain terms: use it for anything, including at work. If you distribute a
modified version, its source has to stay available under these same terms, so
whatever you improve stays improvable by everyone else.
