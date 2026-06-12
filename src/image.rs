use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::host::orchestrator;
use crate::utils::{cleanup, execute_command};

const LAYERS_DIR: &str = "/var/lib/rcr/layers";

pub fn layer_id(parent_id: &str, cur_command: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(parent_id);
    hasher.update(cur_command);

    let digest = hasher.finalize();
    hex::encode(digest)
}

pub fn commit(upper: &str, id: &str) -> Result<()> {
    let dest = format!("{LAYERS_DIR}/{id}");

    // already cached
    if fs::metadata(&dest).is_ok() {
        return Ok(());
    }

    fs::create_dir_all(LAYERS_DIR).context("failed to create layers dir")?;

    let status = Command::new("cp")
        .args(["-a", upper, &dest])
        .status()
        .context("cp failed")?;
    if !status.success() {
        bail!("failed to copy upper into layer {id}");
    }

    Ok(())
}

pub fn pull_image(distro: &str) -> Result<()> {
    let url = match distro {
        "alpine" => {
            "https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/alpine-minirootfs-3.21.0-aarch64.tar.gz"
        }
        "ubuntu" => {
            "https://cdimage.ubuntu.com/ubuntu-base/releases/22.04/release/ubuntu-base-22.04.4-base-arm64.tar.gz"
        }
        _ => bail!("unsupported distro: {distro}"),
    };
    let dest = format!("/var/lib/rcr/bases/{distro}");
    fs::create_dir_all(&dest).context("create base dir failed")?;
    execute_command(
        "sh",
        &["-c", &format!("curl -sL {url} | tar -xz -C {dest}")],
    )?;
    Ok(())
}

// Fluent api builder
pub struct Image {
    base: String,
    steps: Vec<Step>,
}

enum Step {
    Run(String),
    Copy(String, String),
    Env(String, String),
}

impl Image {
    pub fn base(base: &str) -> Self {
        Image {
            base: base.to_string(),
            steps: Vec::new(),
        }
    }

    pub fn run(mut self, command: &str) -> Self {
        self.steps.push(Step::Run(command.to_string()));
        self
    }

    pub fn copy(mut self, from: &str, to: &str) -> Self {
        self.steps
            .push(Step::Copy(from.to_string(), to.to_string()));
        self
    }

    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.steps
            .push(Step::Env(key.to_string(), value.to_string()));
        self
    }

    pub fn build(self, image_name: &str) -> Result<()> {
        let base_path = format!("/var/lib/rcr/bases/{}", self.base);
        if !Path::new(&base_path).exists() {
            pull_image(&self.base)?;
        }

        let mut lower = base_path;
        let mut parent_id = self.base.clone();

        for (i, step) in self.steps.iter().enumerate() {
            let step_hash_key = match step {
                Step::Run(str) => format!("run:{str}"),
                Step::Copy(from, to) => format!("copy:{from}:{to}"),
                Step::Env(key, val) => format!("env:{key}:{val}"),
            };

            let id = layer_id(&parent_id, &step_hash_key);

            let layer_path = format!("{LAYERS_DIR}/{id}");
            if fs::metadata(&layer_path).is_err() {
                match step {
                    Step::Run(command) => {
                        let build_name = format!("build-{image_name}-{i}");
                        run_build_step(&build_name, &lower, command)?;

                        let upper = format!("/run/rcr/{build_name}/upper");
                        commit(&upper, &id)?;
                        cleanup(&build_name);
                    }
                    Step::Copy(from, to) => {
                        let container_dest = format!("{layer_path}/{}", to.trim_start_matches("/"));
                        if let Some(parent) = std::path::Path::new(&container_dest).parent() {
                            fs::create_dir_all(parent)
                                .context("create container dest dir failed")?;
                        }
                        fs::copy(from, container_dest).context("Copy failed")?;
                    }
                    Step::Env(key, val) => {
                        fs::create_dir_all(&layer_path).context("create env layer failed")?;
                        let env_file = format!("{layer_path}/etc");
                        fs::create_dir_all(&env_file).context("create etc dir failed")?;
                        fs::write(format!("{env_file}/rcr-env"), format!("{key}={val}\n"))
                            .context("write to env failed")?;
                    }
                }
            }

            lower = format!("{layer_path}:{lower}");
            parent_id = id;
        }

        save_image(image_name, &lower)?;

        Ok(())
    }
}

fn run_build_step(name: &str, lower: &str, command: &str) -> Result<()> {
    let cmd = vec!["/bin/sh".to_string(), "-c".to_string(), command.to_string()];
    orchestrator(name, lower, false, false, cmd)
}

fn save_image(image_name: &str, lower: &str) -> Result<()> {
    let dir = "/var/lib/rcr/images";
    fs::create_dir_all(dir).context("create images dir failed")?;
    fs::write(format!("{dir}/{image_name}"), lower).context("image save failed")?;
    Ok(())
}
