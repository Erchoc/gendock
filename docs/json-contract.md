# JSON response contract (version 1)

Operation results, version information and errors emit one JSON document on stdout. Foreground `server start` is an explicit streaming exception: successful startup/execution/shutdown produce timestamped, leveled text logs, colored on terminals; startup failures produce a JSON error before success logs. `--daemon`/`--register` return JSON control acknowledgements. Help is the deliberate exception: bare `gendock`, bare command groups, `--help` and `help` emit human-readable text with exit 0. The JSON envelope always has `schema_version`, `ok`, `data`, `error`. Success has a non-error result in `data` and `error: null`. Failure has `data: null` and the following error object:

```json
{
  "schema_version": 1,
  "ok": false,
  "data": null,
  "error": {
    "code": "invalid_arguments",
    "message": "Required arguments are missing.",
    "retryable": false,
    "details": {"arguments": ["--prompt <PROMPT>"]},
    "next_steps": [{
      "action": "inspect_help",
      "description": "Inspect accepted commands and arguments, then correct the request.",
      "argv": ["gendock", "--help"]
    }]
  }
}
```

This follows common API practices: a versioned envelope, stable machine-readable codes, concise human explanation, structured details and actionable recovery. It is not advertised as an HTTP RFC 9457 implementation: CLI exit codes are not HTTP status codes.

- `code`: stable identifier; agents branch on it, not on message text.
- `message`: one short sentence, no rendered usage/help blocks.
- `retryable`: whether the unchanged operation can reasonably succeed later. False does not prohibit a corrected request; true is not permission to retry forever.
- `details`: typed contextual fields. Parsing errors preserve argument/value lists; operation errors preserve causes as an array. Never concatenate a stack of causes or rendered terminal output.
- `next_steps`: an array of action identifiers and explanations. Optional `argv` is an argument array, not a shell command; never join it into shell code. Suggestions must not silently delete data or install models.

Current codes: `invalid_arguments` (exit 2); `not_implemented` (exit 3); `path_not_found`, `permission_denied`, `filesystem_error`, `storage_busy`, `storage_error`, `resource_conflict`, `operation_failed` (exit 1). Version and help exit 0; missing required operation arguments remain JSON errors.

JSON requires newlines *inside string values* to be escaped as `\n`. That escaping is correct and must not be globally removed or double-escaped. Generated text, prompts and file names can legitimately contain newlines. Avoid the underlying design mistake: putting a multiline terminal message into `error.message`. Structural information belongs in objects/arrays. One physical output line is framing, not a requirement to flatten user content.

Current model-management codes: `model_not_found`, `invalid_network_config`, `model_access_denied`, `remote_file_not_found`, `download_unavailable`, `download_failed`, `invalid_download_range`, `integrity_check_failed`, `insufficient_disk_space`, `download_busy` and `resource_conflict` (exit 1). Details and next steps describe recovery; see [models.md](models.md).

Current model-management codes additionally include `invalid_model_manifest`, `builtin_model_protected`, `ambiguous_model`, `model_retired` and `model_not_downloadable` (exit 1). User-imported model origin is normalized; retired/no-file models fail before network requests.

Image-generation codes (exit 1): `invalid_image_options`, `unsupported_output_format`, `output_exists`, `model_execution_unsupported`, `model_not_available`, `model_not_installed`, `runtime_not_ready`, `generation_busy`, `model_files_incomplete`, `generation_cancelled`, `generation_failed`, `invalid_image_output`, `export_failed` and `job_not_found`. An export failure may refer to a succeeded job whose original asset remains available. Asynchronous commands use the service and return a queued job ID; no service returns `server_not_running` (exit 1).

Additional image/configuration codes (exit 1): `invalid_config`, `power_management_unsupported`, `input_not_found` and `invalid_input_image`. Configuration results redact `remote.api_key`; `config path` only reports its location. Invalid config mutations preserve the existing file. See [configuration.md](configuration.md). Missing local models include `next_steps[].argv` with an explicit download command; no automatic network transfer or worker launch occurs.

`database_schema_incompatible` (exit 1) identifies an incompatible development database and includes expected/found markers. Before 1.0, gendock does not automatically migrate or reset these databases; preserve data and back up before explicitly rebuilding a development DB. Compatible DBs are reused and fresh DBs use the single schema snapshot.

Service/client codes (exit 1): `invalid_client_name`, `invalid_connection`, `invalid_server_url`, `invalid_server_response`, `unauthorized`, `forbidden`, `server_not_running`, `server_already_running`, `server_start_failed`, `server_start_timeout`, `server_bind_failed`, `server_unreachable`, `server_busy`, `invalid_submission`, `request_too_large`, `upload_too_large`, `upload_not_found`, `queue_full`, `route_not_found`, `model_revision_changed`, `job_ownership_lost`, `not_queued_job`, `invalid_async_output`, `job_wait_timeout`, `job_failed`, `job_not_complete`, `artifact_not_found`, `artifact_integrity_failed` and `generation_interrupted`. A timeout never cancels or resubmits work. `job wait` returning an unsuccessful terminal task is `job_failed`; `job info` successfully querying the same task remains a successful query.

`service_registration_unsupported`, `service_registration_conflict`, `service_registration_failed` and `service_registration_start_failed` identify platform, ownership, launchctl and readiness failures. Registration preserves unrelated files and rolls back new registration files on failure. Daemon startup propagates the child's concrete JSON error (including bind host/port/OS reason), rather than replacing it with `server_start_failed` when available.

HTTP JSON and SSE data frames reuse the version-1 envelope. Handled responses carry `X-Request-ID`; unauthorized responses additionally carry `WWW-Authenticate: Bearer realm="gendockd"`. Access events record status, duration and error code without headers/query/body data. 401/403/other 4xx log WARN and internal 5xx log ERROR. `/file` is binary PNG on success; errors remain JSON. SSE frames use `event: job`, `id: JOB-STATUS`, `data: ENVELOPE` and heartbeat comments. Reconnect sends the current snapshot, without historical replay. Explicit `server logs --follow` emits NDJSON snapshots; ordinary CLI operations emit one document. See [server.md](server.md).

Local replies may expose local paths in details; callers should redact private information before sharing logs. Remote replies omit host paths, private logs and worker metadata. Client export paths are local CLI results. A future change to field types or envelope semantics requires an explicit schema-version decision and compatibility tests.
