# gendock

One CLI for local AI creation.

Gendock brings local AIGC models and inference backends together behind a simple interface. Generate content on your own machine, or share one AI machine across multiple devices. Use it from a terminal, an app, or an AI agent.

- **Local AI:** bring image, text, audio, and video workflows into one tool.
- **Multiple models and backends:** choose a model for your task without learning each backend's commands.
- **Multiple devices:** run models on one Mac and submit tasks from other computers.
- **Standard interfaces:** integrate through JSON CLI output and an authenticated HTTP API. MCP support is planned.

## Current status

Early development, starting with macOS Apple Silicon.

Model management, image generation and editing, and remote task submission work today. Shared tasks support queuing, cancellation, status updates, and result downloads. The first verified model is FLUX.2 Klein 4B, using a private MLX-based runtime.

More models and backends, text, audio, video, automatic runtime installation, and MCP are planned. Model weights and the image runtime must currently be prepared separately. See [feature status](docs/features.md) for details.

## Install

On an Apple Silicon Mac:

```sh
brew tap Erchoc/gendock https://github.com/Erchoc/gendock
brew install Erchoc/gendock/gendock
```

Update:

```sh
brew update
brew upgrade gendock
```

## Use

With a supported model and runtime ready:

```sh
# Create an image
gendock image generate --prompt "A watercolor fox" --out fox.png

# Edit an image
gendock image edit --input fox.png --prompt "Add a snowy forest" --out winter.png

# Find models
gendock model search flux

# Explore commands
gendock --help
```

To share a Mac with other devices, start the service and create a client token:

```sh
gendock server start --daemon --host
gendock server token --name alice
```

Configure the server address and token on each client, then submit tasks remotely. See [shared-device setup and API](docs/server.md).

## Learn more

- [Models and runtime setup](docs/image-generation.md)
- [Configuration](docs/configuration.md)
- [HTTP API](docs/server.md) and [JSON format](docs/json-contract.md)
- [Roadmap](docs/roadmap.md)
- [Contributing](CONTRIBUTING.md)

## License

MIT. Models and third-party components have their own licenses.
