use anyhow::Error;
use argh::FromArgs;
use aws_build::{
    BuildMode, Builder, DEFAULT_CONTAINER_CMD, DEFAULT_RUST_VERSION,
};
use fehler::throws;
use std::env;
use std::path::{Path, PathBuf};

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
    // TODO
}

/// Build a package for deployment to AWS Lambda.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "lambda")]
struct Lambda {
    // TODO
}

#[derive(Debug, FromArgs)]
#[argh(subcommand)]
enum Command {
    Al2(Al2),
    Lambda(Lambda),
}

/// Build the project in a container for deployment to AWS.
#[derive(Debug, FromArgs)]
struct Opt {
    /// change to DIRECTORY before doing anything
    #[argh(option, short = 'C', default = "env::current_dir().unwrap()")]
    directory: PathBuf,

    /// container command (default: docker)
    #[argh(option, default = "DEFAULT_CONTAINER_CMD.into()")]
    container_cmd: String,

    /// rust version (default: latest stable)
    #[argh(option, default = "DEFAULT_RUST_VERSION.into()")]
    rust_version: String,

    /// name of the binary target to build (required if there is more
    /// than one binary target)
    #[argh(option)]
    bin: Option<String>,

    #[argh(subcommand)]
    command: Command,
}

#[throws]
fn main() {
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(log::LevelFilter::Info))?;

    let opt: Opt = argh::from_env();
    let builder = Builder {
        // TODO
        rust_version: opt.rust_version,
        mode: BuildMode::Lambda,
        bin: opt.bin,
        container_cmd: Path::new(&opt.container_cmd).into(),
        project: opt.directory,
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
        let readme = include_str!("../README.md");
        let mut usage = Opt::from_args(&["aws-build"], &["--help"])
            .unwrap_err()
            .output;
        // Remove the "Usage: " prefix which is not in the readme
        usage = usage.replace("Usage: ", "");
        assert!(readme.contains(&usage));
    }
}
