#!/bin/sh
# Install obsidian-tui from a GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/iamrohithrnair/obsidian-tui/main/install.sh | sh
#
# Environment:
#   OTUI_VERSION  version to install, e.g. v0.1.0 (default: latest)
#   OTUI_BIN_DIR  where to put the binary (default: the first writable of
#                 /usr/local/bin, ~/.local/bin)
#
# POSIX sh on purpose: this has to run under dash, ash and busybox as well as
# bash, because the one thing an installer cannot do is need an installer.

set -eu

REPO="iamrohithrnair/obsidian-tui"
VERSION="${OTUI_VERSION:-latest}"

info() { printf '%s\n' "$*" >&2; }
die() {
	printf 'error: %s\n' "$*" >&2
	exit 1
}

need() {
	command -v "$1" >/dev/null 2>&1 || die "this needs $1, which isn't installed"
}

need uname
need tar
need mktemp

if command -v curl >/dev/null 2>&1; then
	fetch() { curl -fsSL "$1" -o "$2"; }
	fetch_stdout() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
	fetch() { wget -qO "$2" "$1"; }
	fetch_stdout() { wget -qO- "$1"; }
else
	die "this needs curl or wget"
fi

# ---- work out which build to fetch -----------------------------------------

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
Darwin) os_name="apple-darwin" ;;
Linux) os_name="unknown-linux-gnu" ;;
MINGW* | MSYS* | CYGWIN*)
	die "on Windows, download the .zip from https://github.com/$REPO/releases/latest"
	;;
*) die "unsupported operating system: $os" ;;
esac

case "$arch" in
arm64 | aarch64) cpu="aarch64" ;;
x86_64 | amd64) cpu="x86_64" ;;
*) die "unsupported architecture: $arch" ;;
esac

# Apple dropped Intel builds; those Macs can still build from source.
if [ "$os_name" = "apple-darwin" ] && [ "$cpu" = "x86_64" ]; then
	die "no prebuilt binary for Intel Macs. Install with:
    cargo install --git https://github.com/$REPO obsidian-tui"
fi

target="${cpu}-${os_name}"

if [ "$VERSION" = "latest" ]; then
	# Resolve through the API rather than the /latest/download redirect so the
	# checksum file and the archive are guaranteed to come from one release.
	VERSION="$(fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" |
		sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
	[ -n "$VERSION" ] || die "could not work out the latest version"
fi

archive="obsidian-tui-${target}.tar.gz"
base="https://github.com/$REPO/releases/download/$VERSION"

# ---- download and verify ----------------------------------------------------

tmp="$(mktemp -d)"
# shellcheck disable=SC2064  # expand tmp now, not at exit
trap "rm -rf '$tmp'" EXIT INT TERM

info "Downloading obsidian-tui $VERSION for ${target}..."
fetch "$base/$archive" "$tmp/$archive" || die "no build for $target in release $VERSION"
fetch "$base/$archive.sha256" "$tmp/$archive.sha256" || die "could not fetch the checksum"

if command -v shasum >/dev/null 2>&1; then
	sha_cmd="shasum -a 256"
elif command -v sha256sum >/dev/null 2>&1; then
	sha_cmd="sha256sum"
else
	sha_cmd=""
fi

if [ -n "$sha_cmd" ]; then
	expected="$(cut -d' ' -f1 <"$tmp/$archive.sha256")"
	actual="$($sha_cmd "$tmp/$archive" | cut -d' ' -f1)"
	[ "$expected" = "$actual" ] || die "checksum mismatch: expected $expected, got $actual"
	info "Checksum verified."
else
	info "warning: no shasum or sha256sum available, skipping verification"
fi

tar -xzf "$tmp/$archive" -C "$tmp"

# ---- install ----------------------------------------------------------------

if [ -n "${OTUI_BIN_DIR:-}" ]; then
	bin_dir="$OTUI_BIN_DIR"
	mkdir -p "$bin_dir" || die "cannot create $bin_dir"
else
	bin_dir=""
	for candidate in /usr/local/bin "$HOME/.local/bin"; do
		if [ -d "$candidate" ] && [ -w "$candidate" ]; then
			bin_dir="$candidate"
			break
		fi
	done
	if [ -z "$bin_dir" ]; then
		bin_dir="$HOME/.local/bin"
		mkdir -p "$bin_dir" || die "cannot create $bin_dir"
	fi
fi

install -m 755 "$tmp/obsidian-tui-${target}/obsidian-tui" "$bin_dir/obsidian-tui" ||
	die "could not write to $bin_dir; set OTUI_BIN_DIR to somewhere writable"

# Gatekeeper quarantines anything downloaded, and the error it produces
# ("cannot be opened because the developer cannot be verified") gives no hint
# that this is the fix.
if [ "$os" = "Darwin" ] && command -v xattr >/dev/null 2>&1; then
	xattr -d com.apple.quarantine "$bin_dir/obsidian-tui" 2>/dev/null || true
fi

info ""
info "Installed obsidian-tui $VERSION to $bin_dir/obsidian-tui"

case ":$PATH:" in
*":$bin_dir:"*) ;;
*)
	info ""
	info "$bin_dir is not on your PATH. Add it with:"
	info "    echo 'export PATH=\"$bin_dir:\$PATH\"' >> ~/.zshrc"
	;;
esac

info ""
info "Get started:"
info "    obsidian-tui ~/Notes        # open a vault"
info "    obsidian-tui --list-vaults  # vaults Obsidian knows about"
