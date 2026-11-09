use crate::{
    cli::{Command, Job, Model},
    paths::Paths,
    store::Store,
};
use anyhow::{Context, Result};
use serde_json::{Value, json};

pub struct Outcome {
    pub value: Value,
    pub exit: u8,
    pub emit: bool,
}

fn pending(command: &str) -> Outcome {
    Outcome {
        value: crate::cli::output::failure(
            "not_implemented",
            "This command is not implemented in this release.",
            false,
            json!({"command":command}),
            vec![crate::cli::output::NextStep::manual(
                "use_supported_command",
                "Consult docs/commands.md for implemented operations. Retrying the same request will not enable this feature.",
            )],
        ),
        exit: 3,
        emit: true,
    }
}

pub fn execute(command: Command) -> Result<Outcome> {
    execute_at(command, None)
}
pub fn execute_at(command: Command, url: Option<String>) -> Result<Outcome> {
    let foreground = matches!(
        &command,
        Command::Server {
            command: crate::cli::Server::Start {
                daemon: false,
                register: false,
                ..
            }
        }
    );
    // Stubs must not create a database or inspect a user's model files.
    let data = match command {
        Command::Server { command } => execute_server(command, url.as_deref())?,
        Command::Config { command } => crate::config::execute(&Paths::discover()?, command)?,
        Command::Model { command } => match command {
            Model::Add {
                name,
                path,
                backend,
                manifest,
                model_type,
                homepage,
            } => {
                let store = store()?;
                if let Some(path) = manifest {
                    let model = read_manifest(&path)?;
                    store.import_model(&model, false)?;
                    json!({"id":model.id,"origin":"user","catalog_updated":true,"files_uploaded":false})
                } else {
                    let name = name.context("name is required")?;
                    let known = store
                        .catalog_search("")?
                        .into_iter()
                        .find(|model| model.id == name);
                    let backend = backend
                        .or_else(|| known.as_ref().map(|model| model.backend.clone()))
                        .unwrap_or_else(|| "unconfigured".into());
                    let model_type = model_type
                        .or_else(|| known.as_ref().map(|model| model.model_type.clone()))
                        .unwrap_or_else(|| "other".into());
                    store.add_local_model(
                        &name,
                        &path.context("path is required")?,
                        &backend,
                        &model_type,
                        homepage.as_deref(),
                    )?;
                    json!({"name":name,"ownership":"reference","validated":false})
                }
            }
            Model::List => serde_json::to_value(store()?.models()?)?,
            Model::Remove { name, model } => {
                if let Some(name) = name {
                    store()?.remove_model(&name)?;
                    json!({"name":name,"files_deleted":false,"catalog_removed":false})
                } else {
                    let id = model.context("model is required")?;
                    store()?.remove_catalog_model(&id)?;
                    json!({"id":id,"files_deleted":false,"catalog_removed":true})
                }
            }
            Model::Search { query, query_flag } => {
                crate::models::search(&query.or(query_flag).context("query is required")?)?
            }
            Model::Info { model } => crate::models::info(&model)?,
            Model::Update {
                name,
                path,
                manifest,
            } => {
                if let Some(path) = manifest {
                    let model = read_manifest(&path)?;
                    store()?.import_model(&model, true)?;
                    json!({"id":model.id,"origin":"user","catalog_updated":true,"files_modified":false})
                } else {
                    let name = name.context("name is required")?;
                    store()?.update_reference(&name, &path.context("path is required")?)?;
                    json!({"name":name,"reference_updated":true,"files_modified":false})
                }
            }
            Model::Download {
                model,
                endpoint,
                proxy,
                models_dir,
            } => {
                let model = crate::models::find(&model)?;
                let hub = crate::models::hub::Hub::new(&endpoint, proxy.as_deref())?;
                crate::models::download::install(&model, &hub, &Paths::discover()?, models_dir)?
            }
            Model::Load { .. } => return Ok(pending("model load")),
            Model::Unload { .. } => return Ok(pending("model unload")),
        },
        Command::Image {
            command:
                crate::cli::Image::Generate {
                    options,
                    aspect,
                    width,
                    height,
                    steps,
                },
        } => {
            let asynchronous = options.asynchronous;
            let options = crate::image::Options {
                prompt: options.prompt,
                model: options.model,
                out: options.out,
                seed: options.seed,
                aspect,
                width,
                height,
                steps,
            };
            if let Some(connection) = crate::client::discover(url.as_deref())? {
                crate::client::image_request(&connection, options, None, asynchronous)?
            } else if asynchronous {
                return Err(crate::server::error(
                    "server_not_running",
                    "Asynchronous execution requires a running service.",
                    "Run gendock server start first.",
                    json!({}),
                ));
            } else {
                crate::image::generate(options)?
            }
        }
        Command::Image {
            command:
                crate::cli::Image::Edit {
                    input,
                    options,
                    aspect,
                    width,
                    height,
                    steps,
                },
        } => {
            let asynchronous = options.asynchronous;
            let options = crate::image::Options {
                prompt: options.prompt,
                model: options.model,
                out: options.out,
                seed: options.seed,
                aspect,
                width,
                height,
                steps,
            };
            if let Some(connection) = crate::client::discover(url.as_deref())? {
                crate::client::image_request(&connection, options, Some(input), asynchronous)?
            } else if asynchronous {
                return Err(crate::server::error(
                    "server_not_running",
                    "Asynchronous execution requires a running service.",
                    "Run gendock server start first.",
                    json!({}),
                ));
            } else {
                crate::image::edit(options, input)?
            }
        }
        Command::Job { command } => {
            let connection = crate::client::discover(url.as_deref())?;
            match command {
                Job::List => {
                    if let Some(c) = connection {
                        c.jobs()?
                    } else {
                        serde_json::to_value(store()?.jobs()?)?
                    }
                }
                Job::Info { id } => {
                    if let Some(c) = connection {
                        c.info(id)?
                    } else {
                        local_job(id)?
                    }
                }
                Job::Wait { id, timeout } => {
                    crate::client::ensure_success(if let Some(c) = connection {
                        c.wait(id, timeout)?
                    } else {
                        wait_local(id, timeout)?
                    })?
                }
                Job::Cancel { id } => {
                    if let Some(c) = connection {
                        c.cancel(id)?
                    } else {
                        store()?
                            .cancel_service(id, "admin", true)?
                            .ok_or_else(|| job_missing(id))?
                    }
                }
                Job::Fetch { id, out } => {
                    if let Some(c) = connection {
                        c.fetch(id, &out)?
                    } else {
                        fetch_local(id, &out)?
                    }
                }
            }
        }
        other => return Ok(pending(command_name(&other))),
    };
    Ok(Outcome {
        value: crate::cli::output::success(data),
        exit: 0,
        emit: !foreground,
    })
}

fn read_manifest(path: &std::path::Path) -> Result<crate::models::Model> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        anyhow::bail!("model manifest exceeds the 1 MiB limit");
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        crate::models::ModelError {
            code: "invalid_model_manifest",
            message: "The model manifest is not valid model JSON.",
            retryable: false,
            next_step: "Check the JSON syntax and required fields described in docs/models.md.",
            details: json!({"line":error.line(),"column":error.column()}),
        }
        .into()
    })
}

fn execute_server(command: crate::cli::Server, url: Option<&str>) -> Result<Value> {
    use crate::cli::Server;
    match command {
        Server::Start {
            host,
            port,
            daemon,
            register,
            ..
        } => crate::server::start(host, port, daemon, register),
        Server::Token { name } => {
            Ok(json!({"name":name,"token":crate::server::auth::token(&Paths::discover()?,&name)?}))
        }
        Server::Unregister => crate::server::launchd::unregister(&Paths::discover()?),
        Server::Status => {
            if let Some(connection) = crate::client::discover(url)? {
                connection.status()
            } else {
                Ok(json!({"running":false}))
            }
        }
        Server::Stop => {
            if let Some(connection) = crate::client::discover(url)? {
                let response = connection.shutdown()?;
                if url.is_none() && std::env::var_os("GENDOCK_URL").is_none() {
                    let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
                    while std::time::Instant::now() < end && connection.status().is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                Ok(response)
            } else {
                Ok(json!({"running":false}))
            }
        }
        Server::Logs { follow } => {
            let paths = Paths::discover()?;
            if !follow {
                crate::server::logs(&paths)
            } else {
                let signals = crate::runtime::mflux::Signals::install()?;
                let mut previous = Value::Null;
                while !signals.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    let current = crate::server::logs(&paths)?;
                    if current != previous {
                        crate::cli::output::emit(&crate::cli::output::success(current.clone()));
                        previous = current;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                Ok(json!({"following":false}))
            }
        }
    }
}
fn job_missing(id: i64) -> anyhow::Error {
    crate::server::error(
        "job_not_found",
        "The job does not exist.",
        "Use job list to find an existing ID.",
        json!({"job_id":id}),
    )
}
fn local_job(id: i64) -> Result<Value> {
    store()?.job_info(id)?.ok_or_else(|| job_missing(id))
}
fn wait_local(id: i64, timeout: u64) -> Result<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
    loop {
        let job = local_job(id)?;
        if crate::client::terminal(&job) {
            return Ok(job);
        }
        if std::time::Instant::now() >= deadline {
            return Err(crate::server::error(
                "job_wait_timeout",
                "The job is still pending.",
                "Start the service if stopped, then wait again using the same job ID.",
                json!({"job_id":id}),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}
fn fetch_local(id: i64, directory: &std::path::Path) -> Result<Value> {
    let job = crate::client::ensure_success(local_job(id)?)?;
    let path = store()?
        .asset_file(id)?
        .context("completed asset missing")?;
    let bytes = std::fs::read(path)?;
    use sha2::{Digest, Sha256};
    anyhow::ensure!(
        format!("{:x}", Sha256::digest(&bytes))
            == job["result"]["asset"]["sha256"].as_str().unwrap_or(""),
        "asset checksum mismatch"
    );
    let directory = if directory.is_absolute() {
        directory.to_owned()
    } else {
        std::env::current_dir()?.join(directory)
    };
    std::fs::create_dir_all(&directory)?;
    let path = crate::image::export_default(&bytes, &directory, &format!("job_{id}"))?;
    Ok(json!({"job_id":id,"path":path,"bytes":bytes.len()}))
}
fn store() -> Result<Store> {
    Store::open(&Paths::discover()?.database())
}

fn command_name(command: &Command) -> &'static str {
    use crate::cli::*;
    match command {
        Command::Config { .. } => "config",
        Command::Server { command } => match command {
            Server::Token { .. } => "server token",
            Server::Start { .. } => "server start",
            Server::Unregister => "server unregister",
            Server::Stop => "server stop",
            Server::Status => "server status",
            Server::Logs { .. } => "server logs",
        },
        Command::Text { command } => match command {
            Text::Generate(_) => "text generate",
            Text::Chat { .. } => "text chat",
        },
        Command::Image { command } => match command {
            Image::Generate { .. } => "image generate",
            Image::Edit { .. } => "image edit",
            Image::Upscale { .. } => "image upscale",
            Image::RemoveBg { .. } => "image remove-bg",
        },
        Command::Video { command } => match command {
            Video::Generate { .. } => "video generate",
            Video::Extend { .. } => "video extend",
            Video::Upscale { .. } => "video upscale",
        },
        Command::Audio { command } => match command {
            Audio::Speak { .. } => "audio speak",
            Audio::Transcribe { .. } => "audio transcribe",
            Audio::Generate { .. } => "audio generate",
        },
        Command::Music { .. } => "music generate",
        Command::Job { command } => match command {
            Job::Info { .. } => "job info",
            Job::Wait { .. } => "job wait",
            Job::Cancel { .. } => "job cancel",
            Job::Fetch { .. } => "job fetch",
            Job::List => "job list",
        },
        Command::Model { .. } => "model",
    }
}
