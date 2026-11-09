class Gendock < Formula
  desc "Local-first CLI for unified generative AI workflows"
  homepage "https://github.com/Erchoc/gendock"
  url "https://github.com/Erchoc/gendock/releases/download/v0.1.1/gendock-v0.1.1-aarch64-apple-darwin.tar.gz"
  version "0.1.1"
  sha256 "6cc1054a4364bd9cacd511aa1f3085701824c29aac60416f124d6ea8063cbb62"
  license "MIT"

  depends_on :macos
  depends_on arch: :arm64

  def install
    bin.install "bin/gendock"
  end

  test do
    require "json"
    assert_equal version.to_s, JSON.parse(shell_output("#{bin}/gendock --version"))["data"]["version"]
  end
end
