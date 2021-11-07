use anyhow::Error;
use argh::FromArgs;
use aws_build_lib::{
    BuildMode, Builder, ContainerCommand, DEFAULT_RUST_VERSION,
};
use fehler::throws;
use std::env;
use std::path::PathBuf;

use log::{Level, Metadata, Record};

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            println!("{}", record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

/// Build an executable that can run on Amazon Linux 2.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "al2")]
struct Al2 {
    /// path of the project to build (default: current directory)
    #[argh(positional, default = "env::current_dir().unwrap()")]
    project: PathBuf,
}

/// Build a package for deployment to AWS Lambda.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "lambda")]
struct Lambda {
    /// path of the project to build (default: current directory)
    #[argh(positional, default = "env::current_dir().unwrap()")]
    project: PathBuf,
}

#[derive(Debug, FromArgs)]
#[argh(description = "Build the project in a container for deployment to AWS.

mode: al2 or lambda (for Amazon Linux 2 or AWS Lambda, respectively)
project: path of the project to build (default: current directory)
")]
struct Opt {
    /// container command: docker (default), sudo-docker, or podman
    #[argh(option, default = "ContainerCommand::default()")]
    container_cmd: ContainerCommand,

    /// rust version (default: latest stable)
    #[argh(option, default = "DEFAULT_RUST_VERSION.into()")]
    rust_version: String,

    /// strip debug symbols
    #[argh(switch)]
    strip: bool,

    /// name of the binary target to build (required if there is more
    /// than one binary target)
    #[argh(option)]
    bin: Option<String>,

    /// yum devel package to install in build container
    #[argh(option)]
    package: Vec<String>,

    /// whether to build for Amazon Linux 2 or AWS Lambda
    #[argh(positional)]
    mode: BuildMode,

    /// path of the project to build (default: current directory)
    #[argh(positional, default = "env::current_dir().unwrap()")]
    project: PathBuf,
}

#[throws]
fn main() {
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(log::LevelFilter::Info))?;

    let opt: Opt = argh::from_env();
    let builder = Builder {
        rust_version: opt.rust_version,
        mode: opt.mode,
        bin: opt.bin,
        strip: opt.strip,
        container_cmd: opt.container_cmd,
        project: opt.project,
        packages: opt.package,
    };
    builder.run()?;
}

#[cfg(test)]
mod tests {
    use super::*;
    use argh::FromArgs;

    /// Test that the readme's usage section is up to date
    #[test]
    fn test_readme_usage() {
        let readme = include_str!("../../README.md");
        let mut usage = Opt::from_args(&["aws-build"], &["--help"])
            .unwrap_err()
            .output;
        // Remove the "Usage: " prefix which is not in the readme
        usage = usage.replace("Usage: ", "");
        assert!(readme.contains(&usage));
    }
}
