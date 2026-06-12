use sha2::{Digest, Sha256};
use std::fs;
use std::process::Command;

use crate::host::orchestrator;
use crate::utils::cleanup;

const LAYERS_DIR: &str = "/var/lib/rcr/layers";

pub fn layer_id(parent_id: &str, cur_command: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(parent_id);
    hasher.update(cur_command);

    let digest = hasher.finalize();
    hex::encode(digest)
}

pub fn commit(upper: &str, id: &str) {
    let dest = format!("{LAYERS_DIR}/{id}");

    // already cached
    if fs::metadata(&dest).is_ok() {
        return;
    }

    fs::create_dir_all(LAYERS_DIR).expect("failed to create layers dir");

    let status = Command::new("cp")
        .args(["-a", upper, &dest])
        .status()
        .expect("cp failed");
    if !status.success() {
        panic!("failed to copy upper into layer {id}")
    }
}

// Fluent api builder
pub struct Image {
    base: String,
    steps: Vec<String>,
}

impl Image {
    pub fn base(base: &str) -> Self {
        Image {
            base: base.to_string(),
            steps: Vec::new(),
        }
    }

    pub fn run(mut self, command: &str) -> Self {
        self.steps.push(command.to_string());
        self
    }

    pub fn build(self, image_name: &str) {
        const ROOTFS: &str = "/home/andreastrolle.guest/rootfs";

        let mut lower = ROOTFS.to_string();
        let mut parent_id = self.base.clone();

        for (i, step) in self.steps.iter().enumerate() {
            let id = layer_id(&parent_id, step);

            let layer_path = format!("{LAYERS_DIR}/{id}");
            if fs::metadata(&layer_path).is_err() {
                let build_name = format!("build-{image_name}-{i}");
                run_build_step(&build_name, &lower, step);

                let upper = format!("/run/rcr/{build_name}/upper");
                commit(&upper, &id);
                cleanup(&build_name);
            }

            lower = format!("{layer_path}:{lower}");
            parent_id = id;
        }

        save_image(image_name, &lower);
    }
}

fn run_build_step(name: &str, lower: &str, command: &str) {
    let cmd = vec!["/bin/sh".to_string(), "-c".to_string(), command.to_string()];
    orchestrator(name, lower, false, false, cmd);
}

fn save_image(image_name: &str, lower: &str) {
    let dir = "/var/lib/rcr/images";
    fs::create_dir_all(dir).expect("create images dir failed");
    fs::write(format!("{dir}/{image_name}"), lower).expect("image save failed");
}
