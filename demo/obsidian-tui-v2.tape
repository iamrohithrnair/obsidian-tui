# obsidian-tui launch demo, second cut — the 0.3 features included.
#
# Render:  vhs demo/obsidian-tui-v2.tape        (run from the repo root)
#
# Everything the first cut showed, plus the two things this release added: an
# Excalidraw drawing opened as a drawing, and a picture drawn in the reading
# pane. Written alongside demo/obsidian-tui.tape rather than over it, so the
# first recording stays reproducible.
#
# Both demo-vault/ and demo/home/ are gitignored, so this tape names what it
# expects of them rather than assuming a checkout has it:
#
#   demo-vault/Excalidraw/Drawing 2026-07-17 10.01.54.excalidraw.md
#   demo-vault/Reddit post.md, embedding
#   demo-vault/Pasted image 20260729101852.png
#   demo/home/Library/Application Support/obsidian-tui/config.toml
#
# Appearance mirrors Ghostty's stock defaults, read from
# `ghostty +show-config --default`: JetBrains Mono at 13px on #282c34. VHS
# renders through xterm.js rather than Ghostty itself, so this matches the look
# without being literal Ghostty pixels.
#
# The picture needs a patched VHS, and demo/vhs-image-layer.patch is it.
#
# Stock VHS captures `canvas.xterm-text-layer` and `canvas.xterm-cursor-layer`
# and nothing else, while xterm.js's image addon paints sixel onto a canvas of
# its own — so the picture is dropped and leaves the blank space it reserved.
# That is charmbracelet/vhs#43, open since 2022. The patch composites the image
# layer into each captured frame, which is all it takes; the existing fork in
# that issue also rebuilds ttyd, and that turns out to be unnecessary. Build it
# with:
#
#   git clone https://github.com/charmbracelet/vhs && cd vhs
#   git apply /path/to/demo/vhs-image-layer.patch && go build -o vhs-images .
#
# Recorded against upstream 0a2c15e. Stock vhs still renders this tape, and
# still renders everything except the picture, which comes out blank — set
# `images.protocol = "halfblocks"` in demo/home to get a coarse one instead.

Output demo/out/obsidian-tui-v2.gif
Output demo/out/obsidian-tui-v2.mp4

Set Shell zsh
Set FontFamily "JetBrainsMono Nerd Font Mono"
Set FontSize 13
Set Width 1280
Set Height 720
Set Padding 20
Set Framerate 30
Set TypingSpeed 110ms

Set Theme { "name": "Ghostty", "background": "#282c34", "foreground": "#ffffff", "cursor": "#ffffff", "selection": "#3e4451", "black": "#1d1f21", "red": "#cc6666", "green": "#b5bd68", "yellow": "#f0c674", "blue": "#81a2be", "magenta": "#b294bb", "cyan": "#8abeb7", "white": "#c5c8c6", "brightBlack": "#666666", "brightRed": "#d54e53", "brightGreen": "#b9ca4a", "brightYellow": "#e7c547", "brightBlue": "#7aa6da", "brightMagenta": "#c397d8", "brightCyan": "#70c0b1", "brightWhite": "#eaeaea" }

# --- setup, not recorded -----------------------------------------------------
# `env -u NO_COLOR` matters: crossterm honours NO_COLOR and memoises it on the
# first draw, so inheriting it renders the whole UI without a single color.
# HOME is redirected so the run cannot read or write the real config.
#
# The vault is opened from the explorer rather than with `--note`, because
# `--note` starts with focus in the note pane, where `j` scrolls the note and
# Enter follows the first wikilink instead of walking the file tree.
Hide
Type "clear && env -u NO_COLOR HOME=$PWD/demo/home ./target/release/obsidian-tui ./demo-vault --theme obsidian-dark"
Enter
Sleep 4s

# G jumps to the last row (Welcome) and Enter opens it. `open_note` always moves
# focus to the note pane (app.rs:405) and nothing rebinds Tab to get back out,
# so the command palette's "Reveal note in explorer" is the way to return focus.
# g then puts the cursor back at the top of the tree ready for the walk.
Type "G"
Sleep 400ms
Enter
Sleep 1200ms
Ctrl+P
Sleep 600ms
Type "reveal"
Sleep 600ms
Enter
Sleep 1s
Type "g"
Sleep 400ms
Show

# --- 0:00-0:03  hold on the three panes --------------------------------------
Sleep 3s
Screenshot demo/out/still-v2-three-pane.png

# --- 0:03-0:09  walk down the explorer ---------------------------------------
# Folders start folded, so the walk has to open one rather than just counting
# rows past it — which is the better shot anyway. Two rows down is Projects,
# Space unfolds it, and four more rows is Roadmap, the note with five backlinks
# — so the Ctrl+K shot below has something in it.
Type@450ms "jj"
Sleep 600ms
Space
Sleep 1s
Type@450ms "jjjj"
Sleep 500ms

# --- 0:07-0:09  open the note ------------------------------------------------
Enter
Sleep 2s

# --- 0:09-0:13  cycle the right sidebar to backlinks -------------------------
Ctrl+K
Sleep 2500ms
Screenshot demo/out/still-v2-backlinks.png
# A Screenshot grabs the next frame off the stream rather than the current one,
# so it needs a Sleep after it or the next keypress lands in the captured frame.
Sleep 800ms

# --- 0:13-0:20  the graph, then fit it ---------------------------------------
Ctrl+G
Sleep 5s
Type "f"
Sleep 2s
Screenshot demo/out/still-v2-graph.png
Sleep 1s

# --- 0:20-0:22  back to the note ---------------------------------------------
Escape
Sleep 2s

# --- 0:22-0:28  an Excalidraw drawing, opened as a drawing -------------------
# Reached through the switcher rather than the tree: the file lives in an
# Excalidraw/ folder under a plugin-generated name, and typing four letters
# reads better than five more `j`s. What the pane shows is the scene — boxes,
# arrows and labels drawn as vectors — not the wall of compressed base64 the
# note holds on disk.
Ctrl+O
Sleep 800ms
Type "draw"
Sleep 900ms
Enter
Sleep 3s
Screenshot demo/out/still-v2-drawing.png
Sleep 1500ms

# --- 0:28-0:34  a picture in the reading pane --------------------------------
# `![[Pasted image ...png]]` resolved out of the vault and drawn inline, as
# sixel, at the resolution the terminal can actually show — the text inside the
# screenshot is legible in the recording.
Ctrl+O
Sleep 800ms
Type "reddit"
Sleep 900ms
Enter
Sleep 4s
Screenshot demo/out/still-v2-image.png
Sleep 1500ms

# --- 0:34-0:38  recolour to Tokyo Night --------------------------------------
Ctrl+T
Sleep 700ms
Type "tokyo"
Sleep 700ms
Enter
Sleep 1500ms
Screenshot demo/out/still-v2-tokyo.png
Sleep 800ms

# --- 0:38-0:42  end on a light theme -----------------------------------------
Ctrl+T
Sleep 700ms
Type "latte"
Sleep 700ms
Enter
Sleep 1500ms
Screenshot demo/out/still-v2-light.png
Sleep 2s
