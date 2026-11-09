# gendock

An agent-first CLI for local AI creation. Its goal is to curate and package practical mainstream text, image, audio and video solutions so users can work with local models without learning inference engines or environment setup. Ask an agent to create something with gendock, or run the commands yourself. Built with Rust, starting with macOS Apple Silicon.

See [功能进度与完成度](docs/features.md) for available features, readiness percentages and remaining work.

**Early development:** local model management, image generation/editing, and an authenticated shared-device service work. Async clients share a durable FIFO queue with one GPU job at a time, cancellation, restart recovery and completion events. The first recipe uses FLUX.2 Klein 4B; local model files and registered private execution components are required. Automatic component installation, music and video generation remain in development. See [image generation](docs/image-generation.md) and [shared-device service](docs/server.md).

## Install

Install the macOS Apple Silicon binary with Homebrew:

```sh
brew tap Erchoc/gendock https://github.com/Erchoc/gendock
brew install Erchoc/gendock/gendock
# Later:
brew update && brew upgrade gendock
```

The tap formula is generated with the archive SHA-256 at release time. The CLI binary does not bundle model weights or the private inference runtime. Intel Macs, signing and notarization are not currently included. See [release operations](docs/releases.md).

From source, with Rust installed:

```sh
cargo install --path . --locked
```

## Examples

```sh
# Generate an image in the current directory (image_MMDD_HHMM.png)
gendock image generate --prompt "A watercolor fox"

# Choose an output filename
gendock image generate --prompt "A cabin by the sea at sunset" --out cabin.png

# Select a model and make the result reproducible
gendock image generate --model flux2-klein-4b-q4 --prompt "A watercolor fox" --seed 42 --out fox.png

# Edit an existing image, preserving the original
gendock image edit --input cabin.png --prompt "Make it a snowy winter scene"

# Inspect a recorded task
gendock job info --id 1
```

The following interfaces are planned and currently return `not_implemented`:

```sh
gendock music generate --prompt "Warm acoustic folk with vocals" --lyrics-file lyrics.txt --seconds 60 --out song.wav
gendock video generate --image cabin.png --prompt "Waves roll in" --seconds 5 --out cabin.mp4
```

## Shared device

```sh
# On the Mac: start in the background and issue a caller token
 gendock server start --daemon
 gendock server token --name alice

# Submit, then use the returned task ID
 gendock image generate --prompt "A blue cup" --async
 gendock job wait --id 1 --timeout 600
 gendock job fetch --id 1 --out ./results
 gendock server stop
```

`gendock server start` runs in the foreground with colored, leveled logs. Use `--daemon` for background execution or `--register` to install/start the macOS login service `gendockd`; `server unregister` removes that registration. Startup failures remain JSON.

For other computers, configure the listener and set `GENDOCK_URL`/`GENDOCK_TOKEN` on each client. See [service setup and API](docs/server.md) for caller isolation, uploads, recovery and current limits. The client does not need model weights or inference components.

## Defaults

Use `gendock config list/get/set/unset/path` to manage global settings:

```sh
gendock config set output.image /absolute/cloud-sync/images
gendock config set remote.endpoint https://gendock.example.com
gendock config set remote.api_key YOUR_KEY
gendock config set power.prevent_sleep true
```

Images export to the configured directory when `--out` is omitted; the default remains the invocation directory. See [global configuration](docs/configuration.md) for precedence, API key redaction, per-media directories and current limitations.

Edit `~/.config/gendock/config.toml` (or `$XDG_CONFIG_HOME/gendock/config.toml`):

```toml
[defaults]
image_generate = "flux2-klein-4b-q4"
image_edit = "flux2-klein-4b-q4"
```

The file is created on first image execution. `--model` overrides it for one command. Missing local models return an error with a download suggestion; execution never downloads automatically.

## Models

```sh
gendock model search flux
gendock model info --model flux2-klein-4b-q4
gendock model download --model flux2-klein-4b-q4

# Attach existing files to a supported model; no copying or backend selection
gendock model add --name flux2-klein-4b-q4 --path /path/to/model
```

Downloads accept `--endpoint hf-mirror`, `--proxy URL` and `--models-dir PATH`. User definitions can be imported with `model add --manifest FILE`. See [model management](docs/models.md).

Operations return a standard [JSON response](docs/json-contract.md); help is plain text. Use `gendock --help` to discover commands. Data follows XDG directories, including on macOS.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md), [AGENTS.md](AGENTS.md), [architecture](docs/architecture.md) and [roadmap](docs/roadmap.md).

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
./scripts/e2e.sh       # CLI/HTTP/queue pipeline using tiny private workers
./scripts/e2e-real.sh  # Opt-in actual-model generation → editing → fetch
```

See [testing and acceptance](docs/testing.md) for prerequisites, individual test commands and retained real-model artifacts.

## License

MIT. Models and third-party components have their own licenses.
