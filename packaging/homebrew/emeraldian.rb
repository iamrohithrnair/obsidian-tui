# Source of truth for the Homebrew formula.
#
# The release workflow rewrites the version and checksums here, then copies
# this file into the tap repository. Keeping it in the main repo means the
# formula is reviewed alongside the code it installs, and the tap stays a
# generated artifact rather than something maintained by hand.
class Emeraldian < Formula
  desc "Terminal UI for Obsidian vaults, with a graph and an assistant"
  homepage "https://github.com/iamrohithrnair/emeraldian"
  version "0.4.0"
  license "GPL-3.0-or-later"

  on_macos do
    on_arm do
      url "https://github.com/iamrohithrnair/emeraldian/releases/download/v0.4.0/emeraldian-aarch64-apple-darwin.tar.gz"
      sha256 "b912a98127e47df9fae25419b98780e856cc52bcf3a55507c1bd3396f2245d10"
    end
    on_intel do
      # Apple silicon only; Intel Macs build from source.
      odie "emeraldian has no Intel Mac build. Install with: cargo install --git #{homepage} emeraldian"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/iamrohithrnair/emeraldian/releases/download/v0.4.0/emeraldian-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "b314bdddf1e15bba5d4e579d415aa9118dd4827ca856e9b76b051bf2f5fe6b1a"
    end
    on_intel do
      url "https://github.com/iamrohithrnair/emeraldian/releases/download/v0.4.0/emeraldian-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "fdc1b5ef6f96cffc0bb1a8e511013e259692e24e80cb78f819e08a2baa2a0ed8"
    end
  end

  def install
    bin.install "emeraldian"
  end

  def caveats
    <<~EOS
      Open a vault:
        emeraldian ~/Notes

      Or let it find the one Obsidian last had open:
        emeraldian

      On macOS, the first run may ask for permission to read the folder your
      vault lives in. Press ? inside the app for the full key list.
    EOS
  end

  test do
    assert_match "emeraldian #{version}", shell_output("#{bin}/emeraldian --version")
    # Indexing a real vault is the behaviour worth testing, not just --version.
    (testpath/"vault").mkpath
    (testpath/"vault/Note.md").write("# Note\n\nLinks to [[Other]].\n")
    assert_match "emeraldian", shell_output("#{bin}/emeraldian --help")
  end
end
