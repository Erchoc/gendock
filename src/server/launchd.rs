//! User-level launchd integration. Only the OS-required symlink lives outside XDG.
use super::{Paths, Value, error, json};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant},
};
const LABEL: &str = "gendockd";
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Agent {
    label: String,
    program_arguments: Vec<String>,
    environment_variables: BTreeMap<String, String>,
    run_at_load: bool,
    keep_alive: BTreeMap<String, bool>,
    process_type: String,
    throttle_interval: u32,
    standard_out_path: String,
    standard_error_path: String,
    working_directory: String,
    #[serde(rename = "ExitTimeOut")]
    exit_timeout: u32,
}
struct Plan {
    agent: Agent,
    source: PathBuf,
    link: PathBuf,
    target: String,
}
trait Control {
    fn run(&self, args: &[&str]) -> Result<Output>;
}
struct Launchctl;
impl Control for Launchctl {
    fn run(&self, args: &[&str]) -> Result<Output> {
        Ok(Command::new("/bin/launchctl").args(args).output()?)
    }
}
fn text(path: &Path) -> Result<String> {
    Ok(path
        .to_str()
        .context("launchd paths must be UTF-8")?
        .to_owned())
}
impl Plan {
    fn new(
        paths: &Paths,
        home: &Path,
        exe: &Path,
        host: &str,
        port: u16,
        uid: &str,
    ) -> Result<Self> {
        let mut env = BTreeMap::from([
            ("HOME".into(), text(home)?),
            ("GENDOCK_SERVICE".into(), LABEL.into()),
        ]);
        for (key, path) in [
            ("XDG_CONFIG_HOME", &paths.config),
            ("XDG_DATA_HOME", &paths.data),
            ("XDG_STATE_HOME", &paths.state),
            ("XDG_CACHE_HOME", &paths.cache),
        ] {
            env.insert(
                key.into(),
                text(path.parent().context("XDG parent missing")?)?,
            );
        }
        let log = text(&paths.state.join("logs/gendockd.log"))?;
        Ok(Self {
            agent: Agent {
                label: LABEL.into(),
                program_arguments: vec![
                    text(exe)?,
                    "server".into(),
                    "start".into(),
                    "--host".into(),
                    host.into(),
                    "--port".into(),
                    port.to_string(),
                ],
                environment_variables: env,
                run_at_load: true,
                keep_alive: BTreeMap::from([("SuccessfulExit".into(), false)]),
                process_type: "Background".into(),
                throttle_interval: 10,
                standard_out_path: log.clone(),
                standard_error_path: log,
                working_directory: text(home)?,
                exit_timeout: 20,
            },
            source: paths.config.join("launchd/gendockd.plist"),
            link: home.join("Library/LaunchAgents/gendockd.plist"),
            target: format!("gui/{uid}/{LABEL}"),
        })
    }
    fn owned(&self, agent: &Agent) -> bool {
        agent.label == LABEL
            && agent
                .environment_variables
                .get("GENDOCK_SERVICE")
                .is_some_and(|v| v == LABEL)
            && [
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_STATE_HOME",
                "XDG_CACHE_HOME",
            ]
            .iter()
            .all(|k| {
                agent.environment_variables.get(*k) == self.agent.environment_variables.get(*k)
            })
    }
    fn existing(&self) -> Result<Option<Vec<u8>>> {
        if let Ok(meta) = std::fs::symlink_metadata(&self.link)
            && (!meta.file_type().is_symlink() || std::fs::read_link(&self.link)? != self.source)
        {
            return Err(conflict(&self.link));
        }
        match std::fs::read(&self.source) {
            Ok(bytes) => {
                let agent: Agent = plist::from_bytes(&bytes).map_err(|_| conflict(&self.source))?;
                if !self.owned(&agent) {
                    return Err(conflict(&self.source));
                }
                Ok(Some(bytes))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
fn conflict(path: &Path) -> anyhow::Error {
    error(
        "service_registration_conflict",
        "An existing registration belongs to another configuration.",
        "Inspect the existing registration; unregister it from its original XDG configuration before replacing it.",
        json!({"path":path,"service":LABEL}),
    )
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("registration parent missing")?;
    std::fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
fn checked(output: Output, phase: &str) -> Result<()> {
    if !output.status.success() {
        return Err(error(
            "service_registration_failed",
            "launchd could not apply the service registration.",
            "Inspect the structured launchctl diagnostics and the user login session.",
            json!({"phase":phase,"exit_code":output.status.code(),"diagnostics":String::from_utf8_lossy(&output.stderr).chars().take(4096).collect::<String>().lines().collect::<Vec<_>>()}),
        ));
    }
    Ok(())
}
struct Installed {
    previous: Option<Vec<u8>>,
    created_link: bool,
    already_loaded: bool,
    _lock: std::fs::File,
}
fn registration_lock(plan: &Plan) -> Result<std::fs::File> {
    let directory = plan
        .link
        .parent()
        .context("launch agent directory missing")?;
    std::fs::create_dir_all(directory)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(".gendockd.lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    Ok(lock)
}
fn restore(plan: &Plan, state: &Installed) -> Result<()> {
    if state.created_link && std::fs::read_link(&plan.link).is_ok_and(|p| p == plan.source) {
        std::fs::remove_file(&plan.link)?;
    }
    if let Some(bytes) = &state.previous {
        atomic(&plan.source, bytes)?;
    } else if plan.source.exists() {
        std::fs::remove_file(&plan.source)?;
    }
    Ok(())
}
fn install(plan: &Plan, ctl: &impl Control) -> Result<Installed> {
    let lock = registration_lock(plan)?;
    let previous = plan.existing()?;
    let loaded = ctl.run(&["print", &plan.target])?.status.success();
    if loaded {
        let same = previous
            .as_ref()
            .and_then(|bytes| plist::from_bytes::<Agent>(bytes).ok())
            .is_some_and(|old| old == plan.agent);
        if !same {
            return Err(conflict(&plan.link));
        }
        checked(ctl.run(&["kickstart", &plan.target])?, "kickstart")?;
        return Ok(Installed {
            previous,
            created_link: false,
            already_loaded: true,
            _lock: lock,
        });
    }
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &plan.agent)?;
    let mut state = Installed {
        previous,
        created_link: false,
        already_loaded: false,
        _lock: lock,
    };
    atomic(&plan.source, &bytes)?;
    let applied = (|| -> Result<()> {
        std::fs::create_dir_all(plan.link.parent().unwrap())?;
        if std::fs::symlink_metadata(&plan.link).is_err() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&plan.source, &plan.link)?;
            state.created_link = true;
        }
        let domain = plan.target.rsplit_once('/').unwrap().0;
        checked(
            ctl.run(&["bootstrap", domain, &text(&plan.link)?])?,
            "bootstrap",
        )
    })();
    if let Err(error) = applied {
        restore(plan, &state)?;
        return Err(error);
    }
    Ok(state)
}
fn system_plan(paths: &Paths, host: &str, port: u16) -> Result<Plan> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .context("HOME must be absolute")?;
    let uid = Command::new("/usr/bin/id").arg("-u").output()?;
    anyhow::ensure!(uid.status.success(), "cannot determine user ID");
    let uid = String::from_utf8(uid.stdout)?;
    Plan::new(
        paths,
        &home,
        &std::env::current_exe()?,
        host,
        port,
        uid.trim(),
    )
}
pub fn register(paths: &Paths, host: &str, port: u16) -> Result<Value> {
    if !cfg!(target_os = "macos") {
        return Err(error(
            "service_registration_unsupported",
            "Login service registration currently requires macOS.",
            "Use --daemon or an external supervisor on this platform.",
            json!({}),
        ));
    }
    if let Some(receipt) = super::read_receipt(paths)
        && crate::client::Connection::local_from(paths, &receipt).is_ok_and(|c| c.status().is_ok())
    {
        return Err(error(
            "server_already_running",
            "Stop the running service before registering it.",
            "Run gendock server stop, then register the login service.",
            json!({"url":receipt.url}),
        ));
    }
    // Catch port conflicts before installing a persistent, repeatedly failing launch job.
    drop(std::net::TcpListener::bind((host, port)).map_err(|cause| {
        error(
            "server_bind_failed",
            "The service cannot bind its address.",
            "Choose an unused port or use --port 0.",
            json!({"host":host,"port":port,"reason":cause.to_string()}),
        )
    })?);
    let plan = system_plan(paths, host, port)?;
    std::fs::create_dir_all(paths.state.join("logs"))?;
    let ctl = Launchctl;
    let state = install(&plan, &ctl)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(receipt) = super::read_receipt(paths)
            && crate::client::Connection::local_from(paths, &receipt).is_ok_and(|c| {
                c.status()
                    .is_ok_and(|s| s["server_id"] == receipt.server_id)
            })
        {
            return Ok(
                json!({"service":LABEL,"registered":true,"running":true,"mode":"launchd","starts_at":"user_login","addresses":super::network::discover(&receipt.host,receipt.port),"url":receipt.url,"pid":receipt.pid,"plist":plan.source,"launch_agent":plan.link}),
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !state.already_loaded {
        let _ = ctl.run(&["bootout", &plan.target]);
        restore(&plan, &state)?;
    }
    Err(error(
        "service_registration_start_failed",
        "The registered service did not become ready.",
        "Inspect the service console log; new registration files were rolled back.",
        json!({"service":LABEL,"log_path":plan.agent.standard_out_path,"existing_registration":state.already_loaded}),
    ))
}
pub fn unregister(paths: &Paths) -> Result<Value> {
    if !cfg!(target_os = "macos") {
        return Err(error(
            "service_registration_unsupported",
            "Login service registration currently requires macOS.",
            "Use the platform's supervisor to remove its service.",
            json!({}),
        ));
    }
    let plan = system_plan(paths, "127.0.0.1", 0)?;
    remove(&plan, &Launchctl)
}
fn remove(plan: &Plan, ctl: &impl Control) -> Result<Value> {
    let _lock = registration_lock(plan)?;
    let existing = plan.existing()?;
    if existing.is_none() {
        return Ok(json!({"service":LABEL,"registered":false}));
    }
    if ctl.run(&["print", &plan.target])?.status.success() {
        checked(ctl.run(&["bootout", &plan.target])?, "bootout")?;
    }
    if std::fs::read_link(&plan.link).is_ok_and(|p| p == plan.source) {
        std::fs::remove_file(&plan.link)?;
    }
    std::fs::remove_file(&plan.source)?;
    Ok(json!({"service":LABEL,"registered":false}))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        os::unix::process::ExitStatusExt,
    };
    struct Fake {
        loaded: Cell<bool>,
        fail: bool,
        calls: RefCell<Vec<Vec<String>>>,
    }
    impl Control for Fake {
        fn run(&self, args: &[&str]) -> Result<Output> {
            self.calls
                .borrow_mut()
                .push(args.iter().map(|v| v.to_string()).collect());
            let success = match args[0] {
                "print" => self.loaded.get(),
                "bootstrap" => {
                    if !self.fail {
                        self.loaded.set(true);
                    }
                    !self.fail
                }
                "bootout" => {
                    self.loaded.set(false);
                    true
                }
                _ => true,
            };
            Ok(Output {
                status: std::process::ExitStatus::from_raw(if success { 0 } else { 256 }),
                stdout: vec![],
                stderr: b"fixture diagnostic".to_vec(),
            })
        }
    }
    fn fixture() -> (tempfile::TempDir, Plan) {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home & 中文");
        let paths = Paths::resolve(&home, |_| None);
        let plan = Plan::new(
            &paths,
            &home,
            Path::new("/private/app & tools/gendock"),
            "127.0.0.1",
            8199,
            "501",
        )
        .unwrap();
        (root, plan)
    }
    #[test]
    fn register_roundtrips_escaped_plist_and_unregisters_only_owned_files() {
        let (_root, plan) = fixture();
        let ctl = Fake {
            loaded: Cell::new(false),
            fail: false,
            calls: RefCell::new(vec![]),
        };
        install(&plan, &ctl).unwrap();
        let parsed: Agent = plist::from_file(&plan.source).unwrap();
        assert_eq!(parsed, plan.agent);
        assert_eq!(std::fs::read_link(&plan.link).unwrap(), plan.source);
        assert_eq!(parsed.label, "gendockd");
        assert!(!parsed.keep_alive["SuccessfulExit"]);
        assert!(!parsed.environment_variables.contains_key("GENDOCK_TOKEN"));
        assert_eq!(parsed.program_arguments[1..3], ["server", "start"]);
        assert!(install(&plan, &ctl).unwrap().already_loaded);
        assert_eq!(remove(&plan, &ctl).unwrap()["registered"], false);
        assert!(!plan.source.exists());
        assert!(std::fs::symlink_metadata(plan.link).is_err());
        assert!(ctl.calls.borrow().iter().any(|a| a[0] == "bootout"));
    }
    #[test]
    fn bootstrap_failure_rolls_back_and_foreign_agents_are_preserved() {
        let (_root, plan) = fixture();
        let ctl = Fake {
            loaded: Cell::new(false),
            fail: true,
            calls: RefCell::new(vec![]),
        };
        assert!(install(&plan, &ctl).is_err());
        assert!(!plan.source.exists());
        assert!(std::fs::symlink_metadata(&plan.link).is_err());
        std::fs::write(&plan.link, b"someone else's service").unwrap();
        assert!(install(&plan, &ctl).is_err());
        assert_eq!(
            std::fs::read(&plan.link).unwrap(),
            b"someone else's service"
        );
        assert!(remove(&plan, &ctl).is_err());
    }
}
