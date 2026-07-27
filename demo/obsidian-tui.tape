# obsidian-tui launch demo — follows the ~30s script in LAUNCH-reddit.md
#
# Render:  vhs demo/obsidian-tui.tape        (run from the repo root)
#
# Appearance mirrors Ghostty's stock defaults, read from
# `ghostty +show-config --default`: JetBrains Mono at 13px on #282c34. VHS
# renders through xterm.js rather than Ghostty itself, so this matches the look
# without being literal Ghostty pixels.

Output demo/out/obsidian-tui.gif
Output demo/out/obsidian-tui.mp4

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
Screenshot demo/out/still-three-pane.png

# --- 0:03-0:07  walk down the explorer ---------------------------------------
# Eight rows lands on Projects/Roadmap, which has five backlinks — so the
# Ctrl+K shot below has something in it.
Type@450ms "jjjjjjjj"
Sleep 500ms

# --- 0:07-0:09  open the note ------------------------------------------------
Enter
Sleep 2s

# --- 0:09-0:13  cycle the right sidebar to backlinks -------------------------
Ctrl+K
Sleep 2500ms
Screenshot demo/out/still-backlinks.png
# A Screenshot grabs the next frame off the stream rather than the current one,
# so it needs a Sleep after it or the next keypress lands in the captured frame.
Sleep 800ms

# --- 0:13-0:20  the graph, then fit it ---------------------------------------
Ctrl+G
Sleep 5s
Type "f"
Sleep 2s
Screenshot demo/out/still-graph.png
Sleep 1s

# --- 0:20-0:22  back to the note ---------------------------------------------
Escape
Sleep 2s

# --- 0:22-0:26  recolour to Tokyo Night --------------------------------------
Ctrl+T
Sleep 700ms
Type "tokyo"
Sleep 700ms
Enter
Sleep 1500ms
Screenshot demo/out/still-tokyo.png
Sleep 800ms

# --- 0:26-0:30  end on a light theme -----------------------------------------
Ctrl+T
Sleep 700ms
Type "latte"
Sleep 700ms
Enter
Sleep 1500ms
Screenshot demo/out/still-light.png
Sleep 2s
