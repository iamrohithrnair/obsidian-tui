#!/usr/bin/env node
"use strict";

// Thin launcher. The real program is a Rust binary shipped in a
// platform-specific optional dependency, so npm downloads exactly one build
// rather than all of them — the same approach esbuild and swc use.

const { spawn } = require("node:child_process");
const path = require("node:path");

const PACKAGES = {
  "darwin-arm64": "obsidian-tui-darwin-arm64",
  "linux-x64": "obsidian-tui-linux-x64",
  "linux-arm64": "obsidian-tui-linux-arm64",
  "win32-x64": "obsidian-tui-win32-x64",
};

const platform = `${process.platform}-${process.arch}`;
const pkg = PACKAGES[platform];

if (!pkg) {
  const hint =
    platform === "darwin-x64"
      ? "Intel Macs have no prebuilt binary. Install with:\n" +
        "    cargo install --git https://github.com/iamrohithrnair/obsidian-tui obsidian-tui"
      : `Supported: ${Object.keys(PACKAGES).join(", ")}`;
  console.error(`obsidian-tui: no build for ${platform}.\n${hint}`);
  process.exit(1);
}

let binary;
try {
  // Resolve the package, then join, rather than resolving the binary path
  // directly: package.json is always resolvable, a stray file may not be.
  const manifest = require.resolve(`${pkg}/package.json`);
  const exe = process.platform === "win32" ? "obsidian-tui.exe" : "obsidian-tui";
  binary = path.join(path.dirname(manifest), "bin", exe);
} catch {
  console.error(
    `obsidian-tui: the platform package ${pkg} is missing.\n` +
      "This usually means the install ran with optional dependencies disabled.\n" +
      "Reinstall without --no-optional, or use one of:\n" +
      "    brew install iamrohithrnair/tap/obsidian-tui\n" +
      "    curl -fsSL https://raw.githubusercontent.com/iamrohithrnair/obsidian-tui/main/install.sh | sh",
  );
  process.exit(1);
}

// `inherit` matters more than usual here: this is a full-screen TUI that needs
// the real terminal for raw mode, resize events and mouse reporting. Piping
// through Node would break all three.
const child = spawn(binary, process.argv.slice(2), { stdio: "inherit" });

child.on("error", (err) => {
  console.error(`obsidian-tui: could not start ${binary}: ${err.message}`);
  process.exit(1);
});

// Forward the signals a terminal app is expected to react to, and mirror the
// child's exit status so shell scripts and `npx` see the truth.
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => child.kill(signal));
}

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 0);
  }
});
