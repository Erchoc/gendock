pub mod output;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Unified generative AI from the command line")]
pub struct Cli {
    #[arg(
        long,
        global = true,
        help = "Connect to a gendock service (token from GENDOCK_TOKEN)"
    )]
    pub url: Option<String>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Inspect and change global configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Manage the local service.
    Server {
        #[command(subcommand)]
        command: Server,
    },
    /// Manage models and local references.
    Model {
        #[command(subcommand)]
        command: Model,
    },
    /// Generate text and conversations.
    Text {
        #[command(subcommand)]
        command: Text,
    },
    /// Generate and edit images.
    Image {
        #[command(subcommand)]
        command: Image,
    },
    /// Generate and process videos.
    Video {
        #[command(subcommand)]
        command: Video,
    },
    /// Generate and transcribe audio.
    Audio {
        #[command(subcommand)]
        command: Audio,
    },
    /// Music creation.
    Music {
        #[command(subcommand)]
        command: Music,
    },
    /// Inspect and manage generation jobs.
    Job {
        #[command(subcommand)]
        command: Job,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Show effective settings (API keys are redacted).
    List,
    /// Show one effective setting (API keys are redacted).
    Get { key: String },
    /// Set a setting; paths must be absolute, booleans/numbers use TOML syntax.
    Set { key: String, value: String },
    /// Remove an override and restore its built-in default.
    Unset { key: String },
    /// Show the configuration file path without creating it.
    Path,
}

#[derive(Subcommand)]
pub enum Server {
    /// Start the service in the foreground.
    Start {
        #[arg(long, num_args=0..=1, default_missing_value="0.0.0.0", help = "Allow LAN access; optionally specify a listen IP")]
        host: Option<String>,
        #[arg(long, help = "Listen port; 0 chooses a free port")]
        port: Option<u16>,
        #[arg(long, help = "Run in the background", conflicts_with = "register")]
        daemon: bool,
        #[arg(long, help = "Register and start the macOS login service gendockd")]
        register: bool,
        #[arg(long, hide = true, conflicts_with_all = ["daemon", "register"])]
        foreground: bool,
    },
    /// Create or retrieve a client access token (local administration).
    Token {
        #[arg(long)]
        name: String,
    },
    /// Remove the macOS login service registration.
    Unregister,
    /// Stop the background service.
    Stop,
    /// Show service status.
    Status,
    /// Read service logs.
    Logs {
        #[arg(long)]
        follow: bool,
    },
}

#[derive(Subcommand)]
pub enum Model {
    /// Import a model manifest or register existing local files.
    Add {
        #[arg(
            long,
            required_unless_present = "manifest",
            conflicts_with = "manifest"
        )]
        name: Option<String>,
        #[arg(
            long,
            required_unless_present = "manifest",
            conflicts_with = "manifest"
        )]
        path: Option<PathBuf>,
        #[arg(long, conflicts_with = "manifest", hide = true)]
        backend: Option<String>,
        #[arg(long, help = "Import a user model definition from a JSON file")]
        manifest: Option<PathBuf>,
        #[arg(long="type", conflicts_with="manifest", value_parser=["text","image","video","audio","music","other"])]
        model_type: Option<String>,
        #[arg(long, conflicts_with = "manifest")]
        homepage: Option<String>,
    },
    /// Search all model types in the local SQLite catalog.
    Search {
        #[arg(
            value_name = "QUERY",
            required_unless_present = "query_flag",
            conflicts_with = "query_flag"
        )]
        query: Option<String>,
        #[arg(long = "query", hide = true)]
        query_flag: Option<String>,
    },
    /// List registered models.
    List,
    /// Show model details.
    Info {
        #[arg(long)]
        model: String,
    },
    /// Update a user manifest or a local reference path.
    Update {
        #[arg(
            long,
            required_unless_present = "manifest",
            conflicts_with = "manifest"
        )]
        name: Option<String>,
        #[arg(
            long,
            required_unless_present = "manifest",
            conflicts_with = "manifest"
        )]
        path: Option<PathBuf>,
        #[arg(long, help = "Replace an existing user model definition")]
        manifest: Option<PathBuf>,
    },
    /// Download a supported model.
    Download {
        #[arg(long)]
        model: String,
        #[arg(
            long,
            default_value = "hf",
            help = "HF endpoint: hf, hf-mirror, or an HTTPS origin"
        )]
        endpoint: String,
        #[arg(long, help = "HTTP/SOCKS proxy URL; does not select a mirror")]
        proxy: Option<String>,
        #[arg(long, help = "Absolute managed model storage root")]
        models_dir: Option<PathBuf>,
    },
    /// Load a model into memory.
    Load {
        #[arg(long)]
        model: String,
    },
    /// Release a loaded model.
    Unload {
        #[arg(long)]
        model: String,
    },
    /// Remove a user definition or local registration; keep all files.
    Remove {
        #[arg(
            long,
            required_unless_present = "model",
            conflicts_with = "model",
            help = "Remove only a local file registration"
        )]
        name: Option<String>,
        #[arg(
            long,
            required_unless_present = "name",
            help = "Remove a user model definition and its registration"
        )]
        model: Option<String>,
    },
}

#[derive(clap::Args)]
#[group(skip)]
pub struct Generate {
    /// Describe the desired result.
    #[arg(long)]
    pub prompt: String,
    /// Override the configured default model.
    #[arg(long)]
    pub model: Option<String>,
    /// Export results to this path.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Submit without waiting for completion.
    #[arg(long = "async")]
    pub asynchronous: bool,
    /// Set a random seed when supported.
    #[arg(long)]
    pub seed: Option<u64>,
}

#[derive(Subcommand)]
pub enum Text {
    /// Generate text from a prompt.
    Generate(Generate),
    /// Send a message in a conversation.
    Chat {
        #[arg(long)]
        message: String,
        #[arg(long)]
        conversation: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum Image {
    /// Generate images from a prompt.
    Generate {
        #[command(flatten)]
        options: Generate,
        #[arg(long,conflicts_with_all=["width","height"],help="Output aspect ratio, such as 16:9")]
        aspect: Option<String>,
        #[arg(long, requires = "height", help = "Image width in pixels")]
        width: Option<u32>,
        #[arg(long, requires = "width", help = "Image height in pixels")]
        height: Option<u32>,
        #[arg(long, help = "Sampling steps (default: 4)")]
        steps: Option<u32>,
    },
    /// Edit an image using instructions.
    Edit {
        #[arg(long, help = "Source PNG, JPEG or WebP image")]
        input: PathBuf,
        #[command(flatten)]
        options: Generate,
        #[arg(long,conflicts_with_all=["width","height"],help="Output aspect ratio, such as 16:9")]
        aspect: Option<String>,
        #[arg(long, requires = "height", help = "Output width in pixels")]
        width: Option<u32>,
        #[arg(long, requires = "width", help = "Output height in pixels")]
        height: Option<u32>,
        #[arg(long, help = "Sampling steps (default: 4)")]
        steps: Option<u32>,
    },
    /// Increase image resolution.
    Upscale {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value_t = 2)]
        scale: u8,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Remove an image background.
    RemoveBg {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum Video {
    /// Generate video from text or an image.
    Generate {
        #[command(flatten)]
        options: Generate,
        #[arg(long)]
        image: Option<PathBuf>,
        #[arg(long, default_value_t = 5)]
        seconds: u32,
    },
    /// Extend an existing video.
    Extend {
        #[arg(long)]
        input: PathBuf,
        #[command(flatten)]
        options: Generate,
        #[arg(long, default_value_t = 5)]
        seconds: u32,
    },
    /// Increase video resolution.
    Upscale {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value_t = 2)]
        scale: u8,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum Audio {
    /// Convert text to speech.
    Speak {
        #[arg(long)]
        text: String,
        #[arg(long)]
        voice: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Transcribe an audio file.
    Transcribe {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        language: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Generate sound effects or ambience.
    Generate {
        #[command(flatten)]
        options: Generate,
        #[arg(long, default_value_t = 10)]
        seconds: u32,
    },
}

#[derive(Subcommand)]
pub enum Music {
    /// Generate music from a description or lyrics.
    Generate {
        #[command(flatten)]
        options: Generate,
        #[arg(long, default_value_t = 30)]
        seconds: u32,
        #[arg(long)]
        instrumental: bool,
        #[arg(long)]
        lyrics_file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum Job {
    /// List recent jobs.
    List,
    /// Show job details.
    Info {
        #[arg(long)]
        id: i64,
    },
    /// Wait for a job to finish.
    Wait {
        #[arg(long)]
        id: i64,
        #[arg(long, default_value_t = 600)]
        timeout: u64,
    },
    /// Cancel a queued or running job.
    Cancel {
        #[arg(long)]
        id: i64,
    },
    /// Export a completed job's files.
    Fetch {
        #[arg(long)]
        id: i64,
        #[arg(long)]
        out: PathBuf,
    },
}
