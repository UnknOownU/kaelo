class Kaelo < Formula
  desc "Intelligent web fetching for AI agents. Local-first, token-aware, learns as it goes."
  homepage "https://github.com/HachemiH/kaelo"
  url "https://github.com/HachemiH/kaelo/archive/refs/tags/v#{version}.tar.gz"
  version "0.1.0"
  license "MIT"
  sha256 "" # Update with actual SHA256 on release

  depends_on "rust" => :build
  depends_on "pkg-config" => :build

  head "https://github.com/HachemiH/kaelo.git", branch: "main"

  def install
    system "cargo", "build", "--release", "--locked"
    bin.install "target/release/kaelo"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/kaelo --version")
    assert_match "kaelo", shell_output("#{bin}/kaelo --help")
  end
end
