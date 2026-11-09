# Global configuration

`gendock config` manages `$XDG_CONFIG_HOME/gendock/config.toml` (normally `~/.config/gendock/config.toml`). Commands return the standard JSON envelope; bare `config` shows help.

```sh
gendock config path
gendock config list
gendock config get defaults.image_generate
gendock config set defaults.image_generate flux2-klein-4b-q4
gendock config set remote.endpoint https://gendock.example.com
gendock config set remote.api_key YOUR_KEY
gendock config set output.image /Users/you/CloudDrive/GenDock/images
gendock config set output.image_edit /Users/you/CloudDrive/GenDock/edits
gendock config set power.prevent_sleep true
gendock config unset output.image_edit
```

`list` and `get` show effective values, including built-in defaults. API keys are always redacted, including mutation results. `unset KEY` restores that key's built-in default. `path` has no filesystem side effects. Other commands initialize an absent configuration without replacing existing settings. Invalid files and invalid updates fail with `invalid_config`; an invalid update leaves the existing file intact. Writes are serialized using a sidecar lock, synchronized and atomically replaced. Managed writes use owner-only file permissions on Unix. Updating rewrites TOML formatting/comments, preserving other settings. Manual editors should avoid editing simultaneously with configuration commands.

| Key | Default | Behavior |
| --- | --- | --- |
| `defaults.image_generate` | `flux2-klein-4b-q4` | Default image generation model; `--model` overrides |
| `defaults.image_edit` | `flux2-klein-4b-q4` | Default editing model; `--model` overrides |
| `remote.endpoint` | unset | HTTP(S) service origin; no embedded credentials, query, fragment or path prefix |
| `remote.api_key` | unset | Optional bearer token; no Authorization header when absent |
| `output.image` | unset | Absolute export directory for generated images and fallback for edited images |
| `output.image_edit` | unset | Absolute directory for edited images |
| `output.video` | unset | Reserved directory preference; video execution remains unimplemented |
| `output.audio` | unset | Reserved directory preference; audio execution remains unimplemented |
| `output.music` | unset | Reserved directory preference; music execution remains unimplemented |
| `power.prevent_sleep` | `false` | On macOS, hold an idle-system-sleep assertion during local image inference; release/reap on completion, errors or cancellation |
| `server.host` | `127.0.0.1` | Service listen IP |
| `server.port` | `9966` | Service listen port; `--port` overrides it and `0` selects an available port |
| `server.max_pending_per_user` | `20` | Pending-job limit, 1–1000 |

Endpoint precedence: `--url`, `GENDOCK_URL`, `remote.endpoint`, local service discovery. Token precedence: `GENDOCK_TOKEN`, `remote.api_key`. A selected remote endpoint that fails does not silently fall back to local execution. The server may require authentication even though configuring an API key is optional.

Export precedence: explicit `--out`, operation-specific directory, image directory, invocation directory. Relative `--out` always resolves against the invocation directory. Configured directories are created at export time, without changing the working directory. Filename collision protection and managed asset history remain in force. Remote exports use the client's configuration, not a server filesystem path. You can point export directories at an existing cloud-sync folder; gendock itself does not provide cloud synchronization. Keep the live SQLite database on local storage.

Sleep prevention applies to the machine performing inference, including service workers. It does not prevent display sleep, change permanent system settings, or promise to keep a closed Mac laptop awake. Other platforms reject enabled sleep prevention explicitly.

Full Chinese/English interface selection is planned, including help, errors and next-step descriptions. There is currently no `language` setting; JSON keys and stable error codes will remain independent of interface language. Additional global preferences should be introduced with their consuming feature rather than accepted as inactive switches.
