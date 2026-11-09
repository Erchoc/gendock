use serde::Serialize;
use serde_json::{Value, json};

pub const SCHEMA_VERSION: u32 = 1;

/// An actionable instruction. `argv` is an argument array, never shell code.
#[derive(Serialize)]
pub struct NextStep {
    pub action: &'static str,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<String>>,
}

impl NextStep {
    pub fn help(mut command: Vec<String>) -> Self {
        command.push("--help".into());
        Self {
            action: "inspect_help",
            description: "Inspect accepted commands and arguments, then correct the request."
                .into(),
            argv: Some(command),
        }
    }
    pub fn manual(action: &'static str, description: &str) -> Self {
        Self {
            action,
            description: description.into(),
            argv: None,
        }
    }
}

pub fn success(data: Value) -> Value {
    json!({"schema_version": SCHEMA_VERSION, "ok": true, "data": data, "error": null})
}

pub fn failure(
    code: &str,
    message: &str,
    retryable: bool,
    details: Value,
    next_steps: Vec<NextStep>,
) -> Value {
    json!({"schema_version":SCHEMA_VERSION,"ok":false,"data":null,"error":{
        "code":code,"message":message,"retryable":retryable,"details":details,"next_steps":next_steps
    }})
}

pub fn operation_error(error: &anyhow::Error) -> Value {
    if let Some(error) = error.downcast_ref::<crate::client::ApiError>() {
        return error.0.clone();
    }
    if let Some(error) = error.downcast_ref::<crate::models::ModelError>() {
        let mut steps = vec![NextStep::manual("resolve_model_error", error.next_step)];
        if error.code == "model_not_installed"
            && let Some(model) = error.details["model"].as_str()
        {
            steps = vec![NextStep {
                action: "download_model",
                description: "Download the selected model explicitly, then rerun the request."
                    .into(),
                argv: Some(vec![
                    "gendock".into(),
                    "model".into(),
                    "download".into(),
                    "--model".into(),
                    model.into(),
                ]),
            }];
        }
        return failure(
            error.code,
            error.message,
            error.retryable,
            error.details.clone(),
            steps,
        );
    }
    let (code, message, retryable, step) = if let Some(sql) =
        error.downcast_ref::<rusqlite::Error>()
    {
        match sql {
            rusqlite::Error::SqliteFailure(e, _)
                if matches!(
                    e.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) =>
            {
                (
                    "storage_busy",
                    "Database is busy.",
                    true,
                    NextStep::manual(
                        "retry_with_backoff",
                        "Wait briefly and retry with a bounded backoff.",
                    ),
                )
            }
            rusqlite::Error::SqliteFailure(e, _)
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                (
                    "resource_conflict",
                    "The operation conflicts with an existing record or storage constraint.",
                    false,
                    NextStep::manual(
                        "check_resource",
                        "Check existing model registrations and choose a unique name when adding a model.",
                    ),
                )
            }
            _ => (
                "storage_error",
                "Database operation failed.",
                false,
                NextStep::manual(
                    "inspect_storage",
                    "Check the data directory permissions and available disk space. Preserve the database and WAL files.",
                ),
            ),
        }
    } else if let Some(io) = error.downcast_ref::<std::io::Error>() {
        match io.kind() {
            std::io::ErrorKind::NotFound => (
                "path_not_found",
                "A required filesystem path does not exist.",
                false,
                NextStep::manual(
                    "correct_path",
                    "Check the supplied path and reconnect any external disk before retrying.",
                ),
            ),
            std::io::ErrorKind::PermissionDenied => (
                "permission_denied",
                "Filesystem access was denied.",
                false,
                NextStep::manual(
                    "check_permissions",
                    "Check ownership and permissions of the referenced path; do not run as root.",
                ),
            ),
            _ => (
                "filesystem_error",
                "A filesystem operation failed.",
                false,
                NextStep::manual(
                    "inspect_filesystem",
                    "Check path availability, permissions and free space.",
                ),
            ),
        }
    } else {
        (
            "operation_failed",
            "The operation could not be completed.",
            false,
            NextStep::manual(
                "inspect_details",
                "Inspect the structured causes and correct the configuration or request before retrying.",
            ),
        )
    };
    let causes = error
        .chain()
        .flat_map(|cause| {
            cause
                .to_string()
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    failure(
        code,
        message,
        retryable,
        json!({"causes":causes}),
        vec![step],
    )
}

pub fn emit(value: &Value) {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{value}");
}
