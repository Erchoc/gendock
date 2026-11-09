# Command contract

See [features.md](features.md) for the canonical feature status/readiness table; this document defines command syntax and availability boundaries.

Operations return JSON by default, including failures and version information. Bare `gendock`, bare command groups, `--help` and `help` print human-readable help and exit 0. There is no `system` group and no `--json` flag. Use `gendock --help` for command discovery and `<group> <command> --help` for parameters. Commands have at most two levels.

| Group | Commands | Availability |
| --- | --- | --- |
| config | list, get, set, unset, path | Implemented; see [configuration.md](configuration.md) |
| server | start, stop, status, logs, token, unregister | Implemented; foreground logs, optional daemon/login registration and authenticated status/shutdown |
| model | add, update, list, remove | Implemented, user manifests and local references; removal preserves files |
| model | search, info, download | Implemented, SQLite catalog and checked direct-file downloads |
| model | load, unload | Not implemented |
| text | generate, chat | Not implemented |
| image | generate, edit | Implemented directly or through the service queue for installed FLUX.2 Klein 4B q4 |
| image | upscale, remove-bg | Not implemented |
| video | generate, extend, upscale | Not implemented |
| audio | speak, transcribe, generate | Not implemented |
| music | generate | Not implemented |
| job | list, info | Implemented; info includes request/result/error |
| job | wait, cancel, fetch | Implemented; SSE/polling, queued/running cancellation and verified image downloads |

The config group adds five configuration operations. Model search reads SQLite across all types: `gendock model search flux` (`--query flux` is retained for compatibility). Search returns type, family, origin, homepage, file download URLs and lifecycle/recommendation status. `model add/update --manifest FILE` maintains user definitions; `model remove --model ID` removes only a user definition and registration. `model remove --name NAME` detaches a local registration. Files are retained in both cases. See [models.md](models.md).

Image generation/editing read separate defaults from XDG `config.toml`; `--model` overrides one invocation. Both hide runtime selection and refuse missing local models without downloading. Editing takes `--input` (PNG/JPEG/WebP) and preserves the original via a recorded snapshot. Both accept `--width`, `--height`, `--steps`, `--aspect`, `--seed`, and PNG `--out`; see [image-generation.md](image-generation.md). `--async` submits to a running service and returns a job ID; without a service it returns `server_not_running`, with no local model execution or download. See [server.md](server.md) for API ownership and restart semantics. Local reference registration infers the component from a known model ID; the old `--backend` option is hidden and retained for advanced compatibility.

Generation uses `--prompt`, optional `--model`, `--out`, `--seed`, and `--async`. Video accepts `--seconds` and generation can accept `--image`. Editing accepts `--input`. See help for each command's exact options. Specific model compatibility, enum/range validation and runtime defaults will be tightened as backends land; parsing a request does not mean the selected backend supports it.

`text chat` is a JSON request with `--message` and optional `--conversation`; interactive terminal chat is deferred. `server logs --follow` emits versioned JSON snapshots as NDJSON until interrupted. Other operation output remains one JSON line. Remote completion events use SSE with versioned JSON in each data frame; see [server.md](server.md).

JSON responses follow [json-contract.md](json-contract.md): `schema_version`, `ok`, `data`, `error`. Errors include stable `code`, concise `message`, `retryable`, structured `details` and actionable `next_steps`. Placeholder errors identify the command in `error.details.command` and exit 3; they must not touch storage or start processes. Invalid operation arguments exit 2; operation errors exit 1. Help-only invocations print text and exit 0; version returns JSON and exits 0. Operation output is one JSON line on stdout, except foreground service execution and explicit log following. `server start` defaults to foreground leveled logs; startup failures remain JSON. `--daemon` and `--register` return JSON control acknowledgements. Reproducible chained CLI acceptance commands are in [testing.md](testing.md).

Do not add placeholder commands that claim success. Replace placeholders with real service operations incrementally and update this table plus tests in the same change.
