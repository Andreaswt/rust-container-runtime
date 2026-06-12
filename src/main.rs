mod cli;
mod container;
mod host;
mod image;
mod state;
mod utils;

use std::path::Path;

use crate::container::run_container;
use crate::host::{list_containers, logs, orchestrator, stop_container};
use crate::image::{Image, pull_image};
use anyhow::{Context, Result};
use clap::Parser;
use cli::{Cli, Commands};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run {
            name,
            detach,
            image,
            command,
        } => {
            let lower = match image {
                Some(img) => std::fs::read_to_string(format!("/var/lib/rcr/images/{img}"))
                    .with_context(|| format!("image {img} not found"))?,
                None => {
                    // Default to alpine
                    let base = "/var/lib/rcr/bases/alpine";
                    if !Path::new(base).exists() {
                        pull_image("alpine")?;
                    }
                    base.to_string()
                }
            };

            orchestrator(&name, &lower, detach, true, command)?;
        }
        Commands::Build { name } => {
            Image::base("alpine")
                .run("apk add --no-cache python3")
                .env("secret", "mysecret123")
                .copy(
                    "/Users/andreastrolle/Documents/Repositories/rust-container-runtime/main.py",
                    "/main.py",
                )
                .build(&name)?;
        }
        Commands::List => list_containers(),
        Commands::Stop { name } => stop_container(&name),
        Commands::Logs { name } => logs(&name),
        Commands::SpawnChildContainer {
            name,
            child_ip,
            lower,
            netns_ready_fd,
            net_configured_fd,
            command,
        } => run_container(
            &name,
            &child_ip,
            &lower,
            netns_ready_fd,
            net_configured_fd,
            command,
        )?,
    }
    Ok(())
}
