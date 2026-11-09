#!/usr/bin/env python3
"""Release orchestration; only publish mutates the remote repository."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile

ROOT = Path(__file__).resolve().parent.parent
STATE = ROOT / 'target/release-plan.json'


def run(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def plan():
    tags = run('git', 'tag', '--list', 'v[0-9]*', '--sort=-version:refname').splitlines()
    tags = [tag for tag in tags if re.fullmatch(r'v\d+\.\d+\.\d+', tag)]
    base = tags[0] if tags else 'origin/main'
    run('git', 'merge-base', '--is-ancestor', 'origin/main', 'HEAD')
    run('git', 'merge-base', '--is-ancestor', base, 'HEAD')
    count = int(run('git', 'rev-list', '--count', f'{base}..HEAD'))
    release = count > 3 or os.environ.get('FORCE') == 'true'
    print(f'{count} commits since {base}; release={release}')
    if 'GITHUB_OUTPUT' in os.environ:
        with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
            output.write(f'release={str(release).lower()}\n')
    if not release:
        return
    manifest = ROOT / 'Cargo.toml'
    text = manifest.read_text()
    current = re.search(r'^version = "(\d+\.\d+\.\d+)"$', text, re.M)[1]
    version = current
    if tags:
        major, minor, patch = map(int, tags[0][1:].split('.'))
        next_version = (major, minor, patch + 1)
        version = '.'.join(map(str, max(tuple(map(int, current.split('.'))), next_version)))
    manifest.write_text(text.replace(f'version = "{current}"', f'version = "{version}"', 1))
    lockfile = ROOT / 'Cargo.lock'
    locktext, replacements = re.subn(
        r'(\[\[package\]\]\nname = "gendock"\nversion = ")[^"]+("\n)',
        lambda match: match[1] + version + match[2], lockfile.read_text(),
    )
    assert replacements == 1, 'expected one gendock package in Cargo.lock'
    lockfile.write_text(locktext)
    STATE.parent.mkdir(exist_ok=True)
    STATE.write_text(json.dumps({'version': version, 'head': run('git', 'rev-parse', 'HEAD')}))


def package():
    version = json.loads(STATE.read_text())['version']
    binary = ROOT / 'target/aarch64-apple-darwin/release/gendock'
    result = json.loads(run(str(binary), '--version'))
    assert result['data']['version'] == version, result
    assert 'arm64' in run('file', str(binary))
    dist = ROOT / 'target/dist'
    dist.mkdir(exist_ok=True)
    archive = dist / f'gendock-v{version}-aarch64-apple-darwin.tar.gz'
    with tarfile.open(archive, 'w:gz') as bundle:
        for path, name in [(binary, 'bin/gendock'), (ROOT / 'LICENSE', 'LICENSE'), (ROOT / 'README.md', 'README.md')]:
            bundle.add(path, arcname=name)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (dist / 'SHA256SUMS').write_text(f'{digest}  {archive.name}\n')
    formula = ROOT / 'Formula/gendock.rb'
    formula.parent.mkdir(exist_ok=True)
    formula.write_text(f'''class Gendock < Formula
  desc "Local-first CLI for unified generative AI workflows"
  homepage "https://github.com/Erchoc/gendock"
  url "https://github.com/Erchoc/gendock/releases/download/v{version}/{archive.name}"
  version "{version}"
  sha256 "{digest}"
  license "MIT"

  depends_on :macos
  depends_on arch: :arm64

  def install
    bin.install "bin/gendock"
  end

  test do
    require "json"
    assert_equal version.to_s, JSON.parse(shell_output("#{{bin}}/gendock --version"))["data"]["version"]
  end
end
''')
    run('ruby', '-c', str(formula))


def publish():
    state = json.loads(STATE.read_text())
    version = state['version']
    # Do not overwrite work pushed while verification/build was running.
    run('git', 'fetch', 'origin', 'main', 'dev')
    assert run('git', 'rev-parse', 'origin/dev') == state['head'], 'dev changed; rerun release'
    run('git', 'merge-base', '--is-ancestor', 'origin/main', 'HEAD')
    run('git', 'config', 'user.name', 'github-actions[bot]')
    run('git', 'config', 'user.email', '41898282+github-actions[bot]@users.noreply.github.com')
    run('git', 'add', 'Cargo.toml', 'Cargo.lock', 'Formula/gendock.rb')
    run('git', 'commit', '--allow-empty', '-m', f'chore: release v{version}')
    tag = f'v{version}'
    run('git', 'tag', tag)
    run('git', 'push', '--atomic', 'origin', 'HEAD:refs/heads/main', 'HEAD:refs/heads/dev', f'refs/tags/{tag}')
    run('gh', 'release', 'create', tag, '--verify-tag', '--title', tag, '--generate-notes',
        *map(str, sorted((ROOT / 'target/dist').iterdir())))


if __name__ == '__main__':
    {'plan': plan, 'package': package, 'publish': publish}[sys.argv[1]]()
