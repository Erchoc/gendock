# macOS releases

The public repository is https://github.com/Erchoc/gendock. Use only `main` (latest released source and Homebrew formula) and `dev` (development). Open changes against `dev`; releases fast-forward both branches to the tested release commit. Do not push independent commits to `main`.

`Release macOS` runs every Sunday at 01:00 UTC / 09:00 Asia/Shanghai on the default branch. It checks out `dev` and counts commits since the latest stable `vMAJOR.MINOR.PATCH` tag. **More than 3 means at least 4 commits.** With no release yet, it counts from `main`. Below the threshold it exits without changing branches, versions or releases. A manual dispatch with `force=true` bootstraps the first release or publishes an urgent fix.

A release validates formatting, clippy, locked tests and the deterministic CLI/HTTP E2E, then builds `aarch64-apple-darwin` on macOS 15. The first release uses Cargo's current version; later releases increment the previous patch unless Cargo already declares a higher stable version. Cargo.toml, Cargo.lock and the tap formula are committed together. The workflow checks for concurrent dev changes and atomically pushes `main`, `dev` and the tag; divergence fails without overwriting other work. Workflow permissions must allow contents writes; branch rules must permit the release bot's fast-forward pushes.

Assets are `gendock-vVERSION-aarch64-apple-darwin.tar.gz` (executable `bin/gendock`, LICENSE, README) and `SHA256SUMS`. `Formula/gendock.rb` is generated with the exact archive hash, so no separate tap repository or token is required. Install/update:

```sh
brew tap Erchoc/gendock https://github.com/Erchoc/gendock
brew install Erchoc/gendock/gendock
brew update
brew upgrade gendock
brew test Erchoc/gendock/gendock
```

Only Apple Silicon macOS CLI binaries are distributed. Model weights, private runtime installation, Intel binaries, code signing and notarization remain outside this release scope. No model downloads occur in release checks. The scheduled job needs GitHub Actions enabled; GitHub may delay cron jobs and disable scheduled workflows after 60 days of repository inactivity.

If publishing assets fails after the atomic branch/tag push, rerunning the whole release would create another version. Recover the existing tag's assets from the failed run's build when available, or rebuild that exact tag and verify its archive hash against the committed formula before uploading. Do not mark a release complete until the assets and Homebrew installation have been verified.

Local checks for the release planner:

```sh
python3 -m unittest discover -s tests -p 'test_release.py'
```
