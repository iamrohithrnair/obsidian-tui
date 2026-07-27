#!/usr/bin/env bash
# Rewrite the Homebrew formula for a new release.
#
#   update-formula.sh <version-without-v> <dir-of-sha256-files> [formula-path]
#
# Split out of the workflow so it can be run and tested locally, which is the
# only way to be sure a release won't ship a formula with stale checksums.

set -euo pipefail

version="${1:?usage: update-formula.sh <version> <checksum-dir> [formula]}"
sums_dir="${2:?usage: update-formula.sh <version> <checksum-dir> [formula]}"
formula="${3:-$(dirname "$0")/obsidian-tui.rb}"

version="${version#v}"

# Reads the digest out of a `<sha>  <filename>` file, tolerating the `*`
# binary-mode marker that sha256sum writes.
digest_for() {
	local target="$1"
	local file="$sums_dir/obsidian-tui-${target}.tar.gz.sha256"
	[ -f "$file" ] || {
		echo "missing checksum file: $file" >&2
		exit 1
	}
	awk '{print $1}' "$file"
}

darwin_arm="$(digest_for aarch64-apple-darwin)"
linux_arm="$(digest_for aarch64-unknown-linux-gnu)"
linux_intel="$(digest_for x86_64-unknown-linux-gnu)"

for sum in "$darwin_arm" "$linux_arm" "$linux_intel"; do
	[ "${#sum}" -eq 64 ] || {
		echo "not a sha256 digest: $sum" >&2
		exit 1
	}
done

python3 - "$formula" "$version" "$darwin_arm" "$linux_arm" "$linux_intel" <<'PY'
import re
import sys

formula, version, darwin_arm, linux_arm, linux_intel = sys.argv[1:6]
text = open(formula, encoding="utf-8").read()

text = re.sub(r'^  version "[^"]*"$', f'  version "{version}"', text, count=1, flags=re.M)
# Every download URL carries the tag, so they all move together.
text = re.sub(r"/download/v[^/]+/", f"/download/v{version}/", text)

# Each sha256 belongs to the URL immediately above it, so rewrite them as pairs
# rather than by position — a reordered formula would otherwise mismatch.
digests = {
    "aarch64-apple-darwin": darwin_arm,
    "aarch64-unknown-linux-gnu": linux_arm,
    "x86_64-unknown-linux-gnu": linux_intel,
}

def replace(match):
    url, gap = match.group(1), match.group(2)
    for target, digest in digests.items():
        if f"obsidian-tui-{target}.tar.gz" in url:
            return f'{url}{gap}sha256 "{digest}"'
    raise SystemExit(f"no checksum known for url: {url}")

text, count = re.subn(
    r'(url "[^"]*")(\s+)sha256 "[0-9a-f]*"', replace, text
)
if count != len(digests):
    raise SystemExit(f"expected {len(digests)} url/sha256 pairs, rewrote {count}")

open(formula, "w", encoding="utf-8").write(text)
print(f"formula updated to {version}")
PY
