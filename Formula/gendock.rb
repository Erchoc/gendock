class Gendock < Formula
  desc "Local-first CLI for unified generative AI workflows"
  homepage "https://github.com/Erchoc/gendock"
  url "https://github.com/Erchoc/gendock/releases/download/v0.1.0/gendock-v0.1.0-aarch64-apple-darwin.tar.gz"
  version "0.1.0"
  sha256 "24805f24d58f9649ad4ac86f4ebe05be852c7ec80cfdead04500485790b0237b"
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
