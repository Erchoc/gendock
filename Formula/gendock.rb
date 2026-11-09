class Gendock < Formula
  desc "Local-first CLI for unified generative AI workflows"
  homepage "https://github.com/Erchoc/gendock"
  url "https://github.com/Erchoc/gendock/releases/download/v0.1.0/gendock-v0.1.0-aarch64-apple-darwin.tar.gz"
  version "0.1.0"
  sha256 "586e87f9798f2bbaa948f7ff597bf188a2a8f16e51d8917edca1babd24f76421"
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
