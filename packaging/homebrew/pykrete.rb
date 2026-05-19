# Homebrew formula for pykrete.
#
# This is the canonical copy. To publish it, create a tap repository named
# `homebrew-pykrete` (so the tap is `amirnaderi93/pykrete`) and copy this
# file to `Formula/pykrete.rb` there.
#
# After each release, refresh `version` and the three `sha256` values. The
# Release workflow attaches a `<tarball>.sha256` file next to every tarball
# — the hash is the first field of that file.
class Pykrete < Formula
  desc "Static schema checking for PySpark dataframes"
  homepage "https://github.com/amirnaderi93/pykrete"
  version "0.1.0"
  license "MIT"

  base = "https://github.com/amirnaderi93/pykrete/releases/download/v#{version}"

  on_macos do
    on_arm do
      url "#{base}/pykrete-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "f6b0ef0e72a36728a7de93784f776ce9a0a4635f36f33bd947a7d70fc03b0d99"
    end
    on_intel do
      url "#{base}/pykrete-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "11b2ea3fbdb22d81806f1c61f86569222e7f39ffdf128bd6325d4388320b339f"
    end
  end

  on_linux do
    on_intel do
      url "#{base}/pykrete-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "3c7a28685c822e866087bc9d15539e9d0ec4069f73cbd644b9a89442cc5e5da3"
    end
  end

  def install
    bin.install "pykrete"
    bin.install "pykrete-lsp"
  end

  test do
    assert_match "pykrete", shell_output("#{bin}/pykrete --help 2>&1")
  end
end
