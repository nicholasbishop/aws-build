mod command;

use anyhow::{anyhow, Context, Error};
use cargo_metadata::MetadataCommand;
use chrono::{Date, Datelike, Utc};
use command::{
    git_checkout, git_clone, git_fetch, git_remote_set_url, git_rev_parse,
    run_cmd,
};
use fehler::{throw, throws};
use log::info;
use sha2::Digest;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::ZipWriter;

pub static DEFAULT_REPO: &str = "https://github.com/softprops/lambda-rust";
pub static DEFAULT_REV: &str = "master";
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

/// Options for running the build.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LambdaBuilder {
    /// The lambda-rust repo URL.
    pub repo: String,

    /// Branch/tag/commit from which to build.
    pub rev: String,

    /// Container command.
    pub container_cmd: String,

    /// Path of the project to build.
    pub project: PathBuf,
}

impl Default for LambdaBuilder {
    fn default() -> Self {
        LambdaBuilder {
            repo: DEFAULT_REPO.into(),
            rev: DEFAULT_REV.into(),
            container_cmd: DEFAULT_CONTAINER_CMD.into(),
            project: PathBuf::default(),
        }
    }
}

impl LambdaBuilder {
    #[throws]
    pub fn run(&self) -> Vec<PathBuf> {
        // Canonicalize the project path. This is necessary for when it's
        // passed as a Docker volume arg.
        let project_path = self.project.canonicalize().context(format!(
            "failed to canonicalize {}",
            self.project.display(),
        ))?;

        // Ensure that the target directory exists. The output directory
        // ("lambda-target") is mounted to /code/target in the container,
        // but we mount /code from the host read-only, so the target
        // subdirectory needs to already exist. Usually the "target"
        // directory will already exist on the host, but won't if "cargo
        // test" or similar hasn't been run yet.
        ensure_dir_exists(&project_path.join("target"))?;

        // Create the output directory if it doesn't already exist.
        let output_dir = project_path.join("lambda-target");
        ensure_dir_exists(&output_dir)?;

        let repo_url = &self.repo;
        let repo_path = output_dir.join("lambda-rust");
        ensure_dir_exists(&repo_path)?;

        if !repo_path.join(".git").exists() {
            // Clone the repo if it doesn't exist
            git_clone(&repo_path, repo_url)?;
        } else {
            // Ensure the remote is set correctly
            git_remote_set_url(&repo_path, repo_url)?;
            // Fetch updates
            git_fetch(&repo_path)?;
        };

        // Check out the specified revision.
        git_checkout(&repo_path, &self.rev)?;

        // Build the container
        let image_tag =
            format!("lambda-build-{:.16}", git_rev_parse(&repo_path, "HEAD")?);
        run_cmd(
            Command::new(&self.container_cmd)
                .current_dir(&repo_path)
                .args(&["build", "--tag", &image_tag, "."]),
        )?;

        let volume = |src: &Path, dst: &Path| {
            let mut s = OsString::new();
            s.push(src);
            s.push(":");
            s.push(dst);
            s
        };
        let volume_read_only = |src, dst| {
            let mut s = volume(src, dst);
            s.push(":ro");
            s
        };

        // Create two cache directories to speed up rebuilds. These are
        // host mounts rather than volumes so that the permissions aren't
        // set to root only.
        let registry_dir = output_dir.join("cargo-registry");
        ensure_dir_exists(&registry_dir)?;
        let git_dir = output_dir.join("cargo-git");
        ensure_dir_exists(&git_dir)?;

        // Run the container
        run_cmd(
            Command::new(&self.container_cmd)
                .args(&["run", "--rm", "--init"])
                .arg("-u")
                .arg(format!(
                    "{}:{}",
                    users::get_current_uid(),
                    users::get_current_gid()
                ))
                // Mount the project directory
                .arg("-v")
                .arg(volume_read_only(&project_path, Path::new("/code")))
                // Mount two Docker volumes to make rebuilds faster
                .arg("-v")
                .arg(volume(&registry_dir, Path::new("/cargo/registry")))
                .arg("-v")
                .arg(volume(&git_dir, Path::new("/cargo/git")))
                // Mount the output target directory
                .arg("-v")
                .arg(volume(&output_dir, Path::new("/code/target")))
                .arg(image_tag),
        )?;

        // Get the binary target names.
        let binaries = get_package_binaries(&project_path)?;

        // Zip each binary and give the zip a unique name. The lambda-rust
        // build already zips the binaries, but the name is just the
        // binary name. It's helpful to have a more specific name so that
        // multiple versions can be uploaded to S3 without overwriting
        // each other. The new name is
        // "<exec-name>-<yyyymmdd>-<exec-hash>.zip".
        let mut zip_names = Vec::new();
        let mut zip_paths = Vec::new();
        for name in binaries {
            let src = output_dir.join("lambda/release").join(&name);
            let contents = fs::read(&src)
                .context(format!("failed to read {}", src.display()))?;
            let dst_name = make_zip_name(&name, &contents, Utc::now().date());
            let dst = output_dir.join(&dst_name);
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

        let latest_path = output_dir.join("latest");
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
