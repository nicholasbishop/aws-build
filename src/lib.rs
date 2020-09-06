#![deny(missing_docs)]

//! Build a Rust project in a container for deployment to either
//! Amazon Linux 2 or AWS Lambda.

use anyhow::{anyhow, Context, Error};
use cargo_metadata::MetadataCommand;
use chrono::{Date, Datelike, Utc};
use docker_command::{BuildOpt, Docker, RunOpt, User, Volume};
use fehler::{throw, throws};
use log::info;
use sha2::Digest;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use zip::ZipWriter;

/// Default rust verison to install.
pub static DEFAULT_RUST_VERSION: &str = "stable";

/// Default container command used to run the build.
pub static DEFAULT_CONTAINER_CMD: &str = "docker";

/// Create directory if it doesn't already exist.
#[throws]
fn ensure_dir_exists(path: &Path) {
    // Ignore the return value since the directory might already exist
    let _ = fs::create_dir(path);
    if !path.is_dir() {
        throw!(anyhow!("failed to create directory {}", path.display()));
    }
}

/// Get the names of all the binaries targets in a project.
#[throws]
fn get_package_binaries(path: &Path) -> Vec<String> {
    let metadata = MetadataCommand::new().current_dir(path).no_deps().exec()?;
    let mut names = Vec::new();
    for package in metadata.packages {
        for target in package.targets {
            if target.kind.contains(&"bin".to_string()) {
                names.push(target.name);
            }
        }
    }
    names
}

/// Write `contents` to `path`.
///
/// This adds the path as context to the error.
#[throws]
fn write_file(path: &Path, contents: &str) {
    fs::write(path, contents)
        .context(format!("failed to write to {}", path.display()))?;
}

#[throws]
fn write_container_files() -> TempDir {
    let tmp_dir = TempDir::new()?;

    let dockerfile = include_str!("container/Dockerfile");
    write_file(&tmp_dir.path().join("Dockerfile"), dockerfile)?;

    let build_script = include_str!("container/build.sh");
    write_file(&tmp_dir.path().join("build.sh"), build_script)?;

    tmp_dir
}

/// Create the unique zip file name.
///
/// The file name is intended to be identifiable, sortable by time,
/// unique, and reasonably short. To make this it includes:
/// - executable name
/// - year, month, and day
/// - first 16 digits of the sha256 hex hash
fn make_zip_name(name: &str, contents: &[u8], when: Date<Utc>) -> String {
    let hash = sha2::Sha256::digest(&contents);
    format!(
        "{}-{}{:02}{:02}-{:.16x}.zip",
        name,
        when.year(),
        when.month(),
        when.day(),
        // The hash is truncated to 16 characters so that the file
        // name isn't unnecessarily long
        hash
    )
}

/// Whether to build for Amazon Linux 2 or AWS Lambda.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BuildMode {
    /// Build for Amazon Linux 2. The result is a standalone binary
    /// that can be copied to (e.g) an EC2 instance running Amazon
    /// Linux 2.
    AmazonLinux2,

    /// Build for AWS Lambda running Amazon Linux 2. The result is a
    /// zip file containing a single "bootstrap" executable.
    Lambda,
}

/// Options for running the build.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Builder {
    /// Rust version to install. Can be anything rustup understands as
    /// a valid version, e.g. "stable" or "1.45.2".
    pub rust_version: String,

    /// Whether to build for Amazon Linux 2 or AWS Lambda.
    pub mode: BuildMode,

    /// Name of the binary target to build. Can be None if the project
    /// only has one binary target.
    pub bin: Option<String>,

    /// Container command. Defaults to "docker", but "podman" should
    /// work as well.
    pub container_cmd: PathBuf,

    /// Path of the project to build.
    pub project: PathBuf,
}

impl Default for Builder {
    fn default() -> Self {
        Builder {
            rust_version: DEFAULT_RUST_VERSION.into(),
            mode: BuildMode::AmazonLinux2,
            bin: None,
            container_cmd: DEFAULT_CONTAINER_CMD.into(),
            project: PathBuf::default(),
        }
    }
}

impl Builder {
    /// Run the build in a container.
    ///
    /// This will produce zip files ready for use with AWS Lambda in
    /// the lambda-target subdirectory, one zip file per binary
    /// target. The lambda-target/latest file will be updated with a
    /// list of the latest zip names.
    ///
    /// Returns the full paths of each zip file.
    #[throws]
    pub fn run(&self) -> Vec<PathBuf> {
        // Canonicalize the project path. This is necessary for when it's
        // passed as a Docker volume arg.
        let project_path = self.project.canonicalize().context(format!(
            "failed to canonicalize {}",
            self.project.display(),
        ))?;

        // Ensure that the target directory exists
        let target_dir = project_path.join("target");
        ensure_dir_exists(&target_dir)?;

        let mode_name = match self.mode {
            BuildMode::AmazonLinux2 => "al2",
            BuildMode::Lambda => "lambda",
        };

        // Build the container
        let from = match self.mode {
            BuildMode::AmazonLinux2 => {
                // https://hub.docker.com/_/amazonlinux
                "amazonlinux:2"
            }
            BuildMode::Lambda => {
                // https://github.com/lambci/docker-lambda#documentation
                "lambci/lambda:build-provided.al2"
            }
        };
        let image_tag =
            format!("aws-build-{}-{}", mode_name, self.rust_version);
        let docker = Docker {
            sudo: false,
            program: self.container_cmd.clone(),
        };
        // TODO
        let tmp_dir = write_container_files()?;
        docker
            .build(BuildOpt {
                build_args: vec![
                    ("FROM_IMAGE".into(), from.into()),
                    ("RUST_VERSION".into(), self.rust_version.clone()),
                ],
                context: tmp_dir.path().into(),
                tag: Some(image_tag.clone()),
                ..Default::default()
            })
            .run()?;

        // Create two cache directories to speed up rebuilds. These are
        // host mounts rather than volumes so that the permissions aren't
        // set to root only.
        let registry_dir =
            target_dir.join(format!("{}-cargo-registry", mode_name));
        ensure_dir_exists(&registry_dir)?;
        let git_dir = target_dir.join(format!("{}-cargo-git", mode_name));
        ensure_dir_exists(&git_dir)?;

        // Get the binary target names.
        let binaries = get_package_binaries(&project_path)?;

        // TODO
        let bin: String = if let Some(bin) = &self.bin {
            bin.clone()
        } else if binaries.len() == 1 {
            binaries[0].clone()
        } else {
            throw!(anyhow!(
                "must specify bin target when package has more than one"
            ));
        };

        // Run the container
        docker
            .run(RunOpt {
                remove: true,
                env: vec![
                    (
                        "TARGET_DIR".into(),
                        Path::new("/code/target").join(mode_name).into(),
                    ),
                    ("BIN_TARGET".into(), bin.into()),
                ],
                init: true,
                user: Some(User::current()),
                volumes: vec![
                    // Mount the project directory
                    Volume {
                        src: project_path,
                        dst: Path::new("/code").into(),
                        ..Default::default()
                    },
                    // Mount two cargo directories to make rebuilds faster
                    Volume {
                        src: registry_dir,
                        dst: Path::new("/cargo/registry").into(),
                        read_write: true,
                        ..Default::default()
                    },
                    Volume {
                        src: git_dir,
                        dst: Path::new("/cargo/git").into(),
                        read_write: true,
                        ..Default::default()
                    },
                    // Mount the output target directory
                    Volume {
                        src: target_dir.clone(),
                        dst: Path::new("/code/target").into(),
                        read_write: true,
                        ..Default::default()
                    },
                ],
                image: image_tag,
                ..Default::default()
            })
            .run()?;

        // Zip each binary and give the zip a unique name. The lambda-rust
        // build already zips the binaries, but the name is just the
        // binary name. It's helpful to have a more specific name so that
        // multiple versions can be uploaded to S3 without overwriting
        // each other. The new name is
        // "<exec-name>-<yyyymmdd>-<exec-hash>.zip".
        let mut zip_names = Vec::new();
        let mut zip_paths = Vec::new();
        for name in binaries {
            let src = target_dir.join("lambda/release").join(&name);
            let contents = fs::read(&src)
                .context(format!("failed to read {}", src.display()))?;
            let dst_name = make_zip_name(&name, &contents, Utc::now().date());
            let dst = target_dir.join(&dst_name);
            zip_names.push(dst_name);
            zip_paths.push(dst.clone());

            // Create the zip file containing just a bootstrap file (the
            // executable)
            info!("writing {}", dst.display());
            let file = fs::File::create(&dst)
                .context(format!("failed to create {}", dst.display()))?;
            let mut zip = ZipWriter::new(file);
            let options = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("bootstrap", options)?;
            zip.write_all(&contents)?;

            zip.finish()?;
        }

        let latest_path = target_dir.join("latest");
        info!("writing {}", latest_path.display());
        fs::write(latest_path, zip_names.join("\n") + "\n")?;

        zip_paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_zip_name() {
        let when = Utc.ymd(2020, 8, 31);
        assert_eq!(
            make_zip_name("testexecutable", "testcontents".as_bytes(), when),
            "testexecutable-20200831-7097a82a108e78da.zip"
        );
    }
}
