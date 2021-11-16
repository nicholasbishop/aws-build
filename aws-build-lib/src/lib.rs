#![deny(missing_docs)]

//! Build a Rust project in a container for deployment to either
//! Amazon Linux 2 or AWS Lambda.

pub use docker_command;

use anyhow::{anyhow, Context, Error};
use cargo_metadata::MetadataCommand;
use docker_command::command_run::{Command, LogTo};
use docker_command::{BuildOpt, Launcher, RunOpt, UserAndGroup, Volume};
use fehler::{throw, throws};
use fs_err as fs;
use log::{error, info};
use sha2::Digest;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use time::{Date, OffsetDateTime};
use zip::ZipWriter;

/// Default rust version to install.
pub static DEFAULT_RUST_VERSION: &str = "stable";

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

#[throws]
fn write_container_files() -> TempDir {
    let tmp_dir = TempDir::new()?;

    let dockerfile = include_str!("container/Dockerfile");
    fs::write(tmp_dir.path().join("Dockerfile"), dockerfile)?;

    let build_script = include_str!("container/build.sh");
    fs::write(tmp_dir.path().join("build.sh"), build_script)?;

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
    when: Date,
) -> String {
    let hash = sha2::Sha256::digest(contents);
    format!(
        "{}-{}-{}{:02}{:02}-{:.16x}",
        mode.name(),
        name,
        when.year(),
        u8::from(when.month()),
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

/// Recursively set the owner of `dir` using the `podman unshare`
/// command. The input `user` is treated as a user (and group)
/// inside the container. This means that an input of "root" is
/// really the current user (from outside the chroot).
#[throws]
fn set_podman_permissions(user: &UserAndGroup, dir: &Path) {
    Command::with_args(
        "podman",
        &["unshare", "chown", "--recursive", &user.arg()],
    )
    .add_arg(dir)
    .run()?;
}

struct ResetPodmanPermissions<'a> {
    user: UserAndGroup,
    dir: &'a Path,
    done: bool,
}

impl<'a> ResetPodmanPermissions<'a> {
    fn new(user: UserAndGroup, dir: &'a Path) -> Self {
        Self {
            dir,
            user,
            done: false,
        }
    }

    /// Reset the permissions if not already done. Calling this is
    /// preferred to waiting for the drop, because the error can be
    /// propagated.
    #[throws]
    fn reset_permissions(&mut self) {
        if !self.done {
            set_podman_permissions(&self.user, self.dir)?;
            self.done = true;
        }
    }
}

impl<'a> Drop for ResetPodmanPermissions<'a> {
    fn drop(&mut self) {
        if let Err(err) = self.reset_permissions() {
            error!("failed to reset permissions: {}", err);
        }
    }
}

struct Container<'a> {
    mode: BuildMode,
    bin: &'a String,
    launcher: &'a Launcher,
    output_dir: &'a Path,
    image_tag: &'a str,
    relabel: Option<Relabel>,

    /// The root of the code that gets mounted in the container. All the
    /// source must live beneath this directory.
    code_root: &'a Path,
}

impl<'a> Container<'a> {
    #[throws]
    fn run(&self) -> PathBuf {
        let mode_name = self.mode.name();

        // Create two cache directories to speed up rebuilds. These are
        // host mounts rather than volumes so that the permissions aren't
        // set to root only.
        let registry_dir = self
            .output_dir
            .join(format!("{}-cargo-registry", mode_name));
        ensure_dir_exists(&registry_dir)?;
        let git_dir = self.output_dir.join(format!("{}-cargo-git", mode_name));
        ensure_dir_exists(&git_dir)?;

        let mut reset_podman_permissions = None;
        if self.launcher.is_podman() {
            // Recursively set the output directory's permissions such
            // that the non-root user in the container owns it.
            set_podman_permissions(&UserAndGroup::current(), self.output_dir)?;

            // Prepare an object to reset the permissions back to the
            // current user. The current user is "root" inside the
            // container, hence the odd-looking input.
            reset_podman_permissions = Some(ResetPodmanPermissions::new(
                UserAndGroup::root(),
                self.output_dir,
            ));
        }

        let mount_options = match self.relabel {
            Some(Relabel::Shared) => vec!["z".to_string()],
            Some(Relabel::Unshared) => vec!["Z".to_string()],
            None => vec![],
        };

        let mut cmd = self.launcher.run(RunOpt {
            remove: true,
            env: vec![
                (
                    "TARGET_DIR".into(),
                    Path::new("/code/target").join(mode_name).into(),
                ),
                ("BIN_TARGET".into(), self.bin.into()),
            ],
            init: true,
            user: Some(UserAndGroup::current()),
            volumes: vec![
                // Mount the code root
                Volume {
                    src: self.code_root.into(),
                    dst: Path::new("/code").into(),
                    read_write: false,
                    options: mount_options.clone(),
                },
                // Mount two cargo directories to make rebuilds faster
                Volume {
                    src: registry_dir,
                    dst: Path::new("/cargo/registry").into(),
                    read_write: true,
                    options: mount_options.clone(),
                },
                Volume {
                    src: git_dir,
                    dst: Path::new("/cargo/git").into(),
                    read_write: true,
                    options: mount_options.clone(),
                },
                // Mount the output target directory
                Volume {
                    src: self.output_dir.into(),
                    dst: Path::new("/code/target").into(),
                    read_write: true,
                    options: mount_options,
                },
            ],
            image: self.image_tag.into(),
            ..Default::default()
        });
        set_up_command(&mut cmd);
        cmd.run()?;

        if let Some(mut resetter) = reset_podman_permissions {
            // Recursively set the output directory's permissions back
            // to the current user.
            resetter.reset_permissions()?;
        }

        // Return the path of the binary that was built
        self.output_dir
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

/// Relabel files before bind-mounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Relabel {
    /// Mount volumes with the `z` option.
    Shared,

    /// Mount volumes with the `Z` option.
    Unshared,
}

/// Output returned from [`Builder::run`] on success.
pub struct BuilderOutput {
    /// Path of the generated file.
    pub real: PathBuf,

    /// Path of the `latest-*` symlink.
    pub symlink: PathBuf,
}

/// Options for running the build.
#[must_use]
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

    /// Container launcher.
    pub launcher: Launcher,

    /// The root of the code that gets mounted in the container. All the
    /// source must live beneath this directory.
    pub code_root: PathBuf,

    /// The project path is the path of the crate to build. It must be
    /// somewhere within the `code_root` directory (or the same path).
    pub project_path: PathBuf,

    /// dev packages to install in container for build
    pub packages: Vec<String>,

    /// Relabel files before bind-mounting (`z` or `Z` volume
    /// option). Warning: this overwrites the current label on files on
    /// the host. Doing this to a system directory like `/usr` could
    /// [break your system].
    ///
    /// [break your system]: https://docs.docker.com/storage/bind-mounts/#configure-the-selinux-label
    pub relabel: Option<Relabel>,
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
    /// The paths of the files are returned.
    #[throws]
    pub fn run(&self) -> BuilderOutput {
        // Canonicalize the input paths. This is necessary for when it's
        // passed as a Docker volume arg.
        let code_root = fs::canonicalize(&self.code_root)?;
        let project_path = fs::canonicalize(&self.project_path)?;
        let relative_project_path = project_path
            .strip_prefix(&code_root)
            .context("project path must be within the code root")?;

        // Ensure that the target directory exists
        let target_dir = project_path.join("target");
        ensure_dir_exists(&target_dir)?;

        let output_dir = target_dir.join("aws-build");
        ensure_dir_exists(&output_dir)?;

        let image_tag = self
            .build_container(relative_project_path)
            .context("container build failed")?;

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
            launcher: &self.launcher,
            output_dir: &output_dir,
            image_tag: &image_tag,
            bin: &bin,
            relabel: self.relabel,
            code_root: &code_root,
        };
        let bin_path = container.run().context("container run failed")?;

        // Optionally strip symbols
        if self.strip {
            strip(&bin_path)?;
        }

        let bin_contents = fs::read(&bin_path)?;
        let base_unique_name = make_unique_name(
            self.mode,
            &bin,
            &bin_contents,
            OffsetDateTime::now_utc().date(),
        );

        let out_path = match self.mode {
            BuildMode::AmazonLinux2 => {
                // Give the binary a unique name so that multiple
                // versions can be uploaded to S3 without overwriting
                // each other.
                let out_path =
                    output_dir.join(self.mode.name()).join(base_unique_name);
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
                    output_dir.join(self.mode.name()).join(&zip_name);

                // Create the zip file containing just a bootstrap
                // file (the executable)
                info!("writing {}", zip_path.display());
                let file = fs::File::create(&zip_path)?;
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
        info!("symlink: {}", symlink_path.display());

        BuilderOutput {
            real: out_path,
            symlink: symlink_path,
        }
    }

    #[throws]
    fn build_container(&self, relative_project_path: &Path) -> String {
        // Build the container
        let from = match self.mode {
            BuildMode::AmazonLinux2 => {
                // https://hub.docker.com/_/amazonlinux
                "docker.io/amazonlinux:2"
            }
            BuildMode::Lambda => {
                // https://github.com/lambci/docker-lambda#documentation
                "docker.io/lambci/lambda:build-provided.al2"
            }
        };
        let image_tag =
            format!("aws-build-{}-{}", self.mode.name(), self.rust_version);
        let tmp_dir = write_container_files()?;
        let mut cmd = self.launcher.build(BuildOpt {
            build_args: vec![
                ("FROM_IMAGE".into(), from.into()),
                ("RUST_VERSION".into(), self.rust_version.clone()),
                ("DEV_PKGS".into(), self.packages.join(" ")),
                (
                    "PROJECT_PATH".into(),
                    relative_project_path
                        .to_str()
                        .ok_or_else(|| anyhow!("project path is not utf-8"))?
                        .into(),
                ),
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
    use time::Month;

    #[test]
    fn test_unique_name() {
        let when = Date::from_calendar_date(2020, Month::August, 31).unwrap();
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
