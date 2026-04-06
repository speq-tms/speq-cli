#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "" ]]; then
  echo "usage: $0 <version-without-v-prefix>"
  echo "example: $0 0.1.0-alpha.0"
  exit 1
fi

version="$1"
repo="${REPO:-speq-tms/speq-cli}"
darwin_arm_sha="${DARWIN_ARM_SHA256:-}"
darwin_amd_sha="${DARWIN_AMD_SHA256:-}"

if [[ -z "$darwin_arm_sha" || -z "$darwin_amd_sha" ]]; then
  echo "set both DARWIN_ARM_SHA256 and DARWIN_AMD_SHA256"
  exit 1
fi

cat <<EOF
class Speq < Formula
  desc "Open-source CLI runtime for speq"
  homepage "https://github.com/${repo}"
  version "${version}"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/${repo}/releases/download/v${version}/speq-darwin-aarch64.tar.gz"
      sha256 "${darwin_arm_sha}"
    else
      url "https://github.com/${repo}/releases/download/v${version}/speq-darwin-x86_64.tar.gz"
      sha256 "${darwin_amd_sha}"
    end
  end

  def install
    bin.install "speq"
  end

  test do
    assert_match "speq", shell_output("#{bin}/speq --help")
  end
end
EOF
