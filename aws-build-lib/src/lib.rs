#![deny(missing_docs)]

//! Build a Rust project in a container for deployment to either
//! Amazon Linux 2 or AWS Lambda.

use anyhow::{anyhow, Context, Error};
use cargo_metadata::MetadataCommand;
use chrono::{Date, Datelike, Utc};
use docker_command::command_run::{Command, LogTo};
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

fn set_up_command(cmd: &mut Command) {
    cmd.log_to = LogTo::Log;
    cmd.combine_output = true;
    cmd.log_output_on_error = true;
}

/// Create a unique output file name.
///
/// The file name is intended to be identifiable, sortable by time,
/// unique, and reasonably short. To make this it includes:
/// - build-mode prefix (al2 or lambda)
/// - executable name
/// - year, month, and day
/// - first 16 digits of the sha256 hex hash
fn make_unique_name(
    mode: BuildMode,
    name: &str,
    contents: &[u8],
    when: Date<Utc>,
) -> String {
    let hash = sha2::Sha256::digest(&contents);
    format!(
        "{}-{}-{}{:02}{:02}-{:.16x}",
        mode.name(),
        name,
        when.year(),
        when.month(),
        when.day(),
        // The hash is truncated to 16 characters so that the file
        // name isn't unnecessarily long
        hash
    )
}

/// Run the strip command to remove symbols and decrease the size.
#[throws]
fn strip(path: &Path) {
    let mut cmd = Command::new("strip");
    cmd.add_arg(path);
    set_up_command(&mut cmd);
    cmd.run()?;
}

struct Container<'a> {
    mode: BuildMode,
    bin: &'a String,
    docker: &'a Docker,
    project_path: &'a Path,
    target_dir: &'a Path,
    image_tag: &'a str,
}

impl<'a> Container<'a> {
    #[throws]
    fn run(&self) -> PathBuf {
        let mode_name = self.mode.name();

        // Create two cache directories to speed up rebuilds. These are
        // host mounts rather than volumes so that the permissions aren't
        // set to root only.
        let registry_dir = self
            .target_dir
            .join(format!("{}-cargo-registry", mode_name));
        ensure_dir_exists(&registry_dir)?;
        let git_dir = self.target_dir.join(format!("{}-cargo-git", mode_name));
        ensure_dir_exists(&git_dir)?;

        let mut cmd = self.docker.run(RunOpt {
            remove: true,
            env: vec![
                (
                    "TARGET_DIR".into(),
                    Path::new("/code/target").join(mode_name).into(),
                ),
                ("BIN_TARGET".into(), self.bin.into()),
            ],
            init: true,
            user: Some(User::current()),
            volumes: vec![
                // Mount the project directory
                Volume {
                    src: self.project_path.into(),
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
                    src: self.target_dir.into(),
                    dst: Path::new("/code/target").into(),
                    read_write: true,
                    ..Default::default()
                },
            ],
            image: self.image_tag.into(),
            ..Default::default()
        });
        set_up_command(&mut cmd);
        cmd.run()?;

        // Return the path of the binary that was built
        self.target_dir
            .join(mode_name)
            .join("release")
            .join(self.bin)
    }
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

impl BuildMode {
    fn name(&self) -> &'static str {
        match self {
            BuildMode::AmazonLinux2 => "al2",
            BuildMode::Lambda => "lambda",
        }
    }
}

impl std::str::FromStr for BuildMode {
    type Err = Error;

    #[throws]
    fn from_str(s: &str) -> Self {
        if s == "al2" {
            Self::AmazonLinux2
        } else if s == "lambda" {
            Self::Lambda
        } else {
            throw!(anyhow!("invalid mode {}", s));
        }
    }
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

    /// Strip the binary.
    pub strip: bool,

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
            strip: false,
            container_cmd: DEFAULT_CONTAINER_CMD.into(),
            project: PathBuf::default(),
        }
    }
}

impl Builder {
    /// Run the build in a container.
    ///
    /// This will produce either a standalone executable (for Amazon
    /// Linux 2) or a zip file (for AWS Lambda). The file is given a
    /// unique name for convenient uploading to S3, and a short
    /// symlink to the file is also created (target/latest-al2 or
    /// target/latest-lambda).
    ///
    /// The full path of the output file (not the symlink) is
    /// returned.
    #[throws]
    pub fn run(&self) -> PathBuf {
        // Canonicalize the project path. This is necessary for when it's
        // passed as a Docker volume arg.
        let project_path = self.project.canonicalize().context(format!(
            "failed to canonicalize {}",
            self.project.display(),
        ))?;

        // Ensure that the target directory exists
        let target_dir = project_path.join("target");
        ensure_dir_exists(&target_dir)?;

        let docker = Docker {
            sudo: false,
            program: self.container_cmd.clone(),
        };
        let image_tag = self.build_container(&docker)?;

        // Get the binary target names
        let binaries = get_package_binaries(&project_path)?;

        // Get the name of the binary target to build
        let bin: String = if let Some(bin) = &self.bin {
            bin.clone()
        } else if binaries.len() == 1 {
            binaries[0].clone()
        } else {
            throw!(anyhow!(
                "must specify bin target when package has more than one"
            ));
        };

        // Build the project in a container
        let container = Container {
            mode: self.mode,
            docker: &docker,
            project_path: &project_path,
            target_dir: &target_dir,
            image_tag: &image_tag,
            bin: &bin,
        };
        let bin_path = container.run()?;

        // Optionally strip symbols
        if self.strip {
            strip(&bin_path)?;
        }

        let bin_contents = fs::read(&bin_path)
            .context(format!("failed to read {}", bin_path.display()))?;
        let base_unique_name =
            make_unique_name(self.mode, &bin, &bin_contents, Utc::now().date());

        let out_path = match self.mode {
            BuildMode::AmazonLinux2 => {
                // Give the binary a unique name so that multiple
                // versions can be uploaded to S3 without overwriting
                // each other.
                let out_path =
                    target_dir.join(self.mode.name()).join(base_unique_name);
                fs::copy(bin_path, &out_path)?;
                info!("writing {}", out_path.display());
                out_path
            }
            BuildMode::Lambda => {
                // Zip the binary and give the zip a unique name so
                // that multiple versions can be uploaded to S3
                // without overwriting each other.
                let zip_name = base_unique_name + ".zip";
                let zip_path =
                    target_dir.join(self.mode.name()).join(&zip_name);

                // Create the zip file containing just a bootstrap
                // file (the executable)
                info!("writing {}", zip_path.display());
                let file = fs::File::create(&zip_path).context(format!(
                    "failed to create {}",
                    zip_path.display()
                ))?;
                let mut zip = ZipWriter::new(file);
                let options = zip::write::FileOptions::default()
                    .unix_permissions(0o755)
                    .compression_method(zip::CompressionMethod::Deflated);
                zip.start_file("bootstrap", options)?;
                zip.write_all(&bin_contents)?;

                zip.finish()?;

                zip_path
            }
        };

        // Create a symlink pointing to the output file. Either
        // "target/latest-al2" or "target/latest-lambda"
        let symlink_path =
            target_dir.join(format!("latest-{}", self.mode.name()));
        // Remove the symlink if it already exists, but ignore an
        // error in case it doesn't exist.
        let _ = fs::remove_file(&symlink_path);
        std::os::unix::fs::symlink(&out_path, &symlink_path)?;

        out_path
    }

    #[throws]
    fn build_container(&self, docker: &Docker) -> String {
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
            format!("aws-build-{}-{}", self.mode.name(), self.rust_version);
        let tmp_dir = write_container_files()?;
        let mut cmd = docker.build(BuildOpt {
            build_args: vec![
                ("FROM_IMAGE".into(), from.into()),
                ("RUST_VERSION".into(), self.rust_version.clone()),
            ],
            context: tmp_dir.path().into(),
            tag: Some(image_tag.clone()),
            ..Default::default()
        });
        set_up_command(&mut cmd);
        cmd.run()?;
        image_tag
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_unique_name() {
        let when = Utc.ymd(2020, 8, 31);
        assert_eq!(
            make_unique_name(
                BuildMode::Lambda,
                "testexecutable",
                "testcontents".as_bytes(),
                when
            ),
            "lambda-testexecutable-20200831-7097a82a108e78da"
        );
    }
}
