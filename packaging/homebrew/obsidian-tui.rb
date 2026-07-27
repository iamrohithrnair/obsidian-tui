# Source of truth for the Homebrew formula.
#
# The release workflow rewrites the version and checksums here, then copies
# this file into the tap repository. Keeping it in the main repo means the
# formula is reviewed alongside the code it installs, and the tap stays a
# generated artifact rather than something maintained by hand.
class ObsidianTui < Formula
  desc "Obsidian-like terminal UI for Markdown vaults, with a graph and an assistant"
  homepage "https://github.com/iamrohithrnair/obsidian-tui"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/iamrohithrnair/obsidian-tui/releases/download/v0.1.0/obsidian-tui-aarch64-apple-darwin.tar.gz"
      sha256 "b49b2c33e89e083d5bc4db4edcae2babce3634d02a6947e7004c40ffbbff6346"
    end
    on_intel do
      # Apple silicon only; Intel Macs build from source.
      odie "obsidian-tui has no Intel Mac build. Install with: cargo install --git #{homepage} obsidian-tui"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/iamrohithrnair/obsidian-tui/releases/download/v0.1.0/obsidian-tui-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "57ec1a2d795fdc7bd0333d89fff04efe4cef56db937915e785f7047f3a138785"
    end
    on_intel do
      url "https://github.com/iamrohithrnair/obsidian-tui/releases/download/v0.1.0/obsidian-tui-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "3ace3d577a20a8d992f8d455f7c1bef7d1bc89e20ba60cb19758290986b92554"
    end
  end

  def install
    bin.install "obsidian-tui"
  end

  def caveats
    <<~EOS
      Open a vault:
        obsidian-tui ~/Notes

      Or let it find the one Obsidian last had open:
        obsidian-tui

      On macOS, the first run may ask for permission to read the folder your
      vault lives in. Press ? inside the app for the full key list.
    EOS
  end

  test do
    assert_match "obsidian-tui #{version}", shell_output("#{bin}/obsidian-tui --version")
    # Indexing a real vault is the behaviour worth testing, not just --version.
    (testpath/"vault").mkpath
    (testpath/"vault/Note.md").write("# Note\n\nLinks to [[Other]].\n")
    assert_match "obsidian-tui", shell_output("#{bin}/obsidian-tui --help")
  end
end
