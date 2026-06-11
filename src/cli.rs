use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rcr")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run a new container
    Run {
        /// Container name
        name: String,
        #[arg(short, long)]
        detach: bool,
        #[arg(trailing_var_arg = true, default_values_t = vec!["/bin/sh".to_string()])]
        command: Vec<String>,
    },
    /// List running containers
    List,
    /// Stop a running container
    Stop {
        /// Container name
        name: String,
    },
    /// Read logs from a detached container
    Logs {
        /// Container name
        name: String,
    },
    /// Internal: the re-exec'd container process (not for direct use)
    #[command(hide = true)]
    SpawnChildContainer {
        name: String,
        child_ip: String,
        netns_ready_fd: i32,
        net_configured_fd: i32,
        #[arg(trailing_var_arg = true)]
        command: Vec<String>,
    },
}
