#![allow(clippy::module_name_repetitions)]

use std::path::PathBuf;

use clap::{AppSettings, Args, Parser, Subcommand};

use tmexclude_lib::rpc::Request;

#[derive(Debug, Parser)]
#[clap(about, version, author, setting(AppSettings::PropagateVersion))]
pub struct Arg {
    #[clap(subcommand)]
    pub command: Command,
    /// Specify the config file. Accepted formats are Json and Toml. Defaults to .tmexclude.yaml in home directory.
    #[clap(short, long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
#[clap(author)]
pub enum Command {
    /// Start the daemon to watch the filesystem continuously.
    Daemon(DaemonArgs),
    /// Perform a full scan and set the exclusion flag to your files.
    Scan(ScanArgs),
    #[clap(flatten)]
    Client(ClientCommand),
}

#[derive(Debug, Subcommand)]
pub enum ClientCommand {
    /// Pause daemon.
    Pause(ClientArgs),
    /// Reload config and restart daemon.
    Reload(ClientArgs),
    /// Restart daemon. This method doesn't reload config.
    Restart(ClientArgs),
    /// Shutdown daemon.
    Shutdown(ClientArgs),
}

impl ClientCommand {
    pub const fn args(&self) -> &ClientArgs {
        match self {
            ClientCommand::Pause(a)
            | ClientCommand::Reload(a)
            | ClientCommand::Restart(a)
            | ClientCommand::Shutdown(a) => a,
        }
    }
}

impl From<&ClientCommand> for Request {
    fn from(cmd: &ClientCommand) -> Self {
        match cmd {
            ClientCommand::Pause(_) => Self::Pause,
            ClientCommand::Reload(_) => Self::Reload,
            ClientCommand::Restart(_) => Self::Restart,
            ClientCommand::Shutdown(_) => Self::Shutdown,
        }
    }
}

#[derive(Debug, Args)]
pub struct DaemonArgs {
    /// Don't touch the system. This flag overrides the config file.
    #[clap(short, long)]
    pub dry_run: bool,
    /// Bind to this Unix domain socket.
    #[clap(short, long)]
    pub uds: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ClientArgs {
    /// Connect through this Unix domain socket.
    #[clap(short, long)]
    pub uds: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// Don't touch the system. This flag overrides the config file.
    #[clap(short, long)]
    pub dry_run: bool,
    /// Bypass any and all confirm messages.
    #[clap(long)]
    pub noconfirm: bool,
    /// Connect to the daemon through the given Unix domain socket.
    #[clap(short, long)]
    pub uds: Option<PathBuf>,
}
