use anyhow::Error;
use argh::FromArgs;
use fehler::throws;
use lambda_build::{
    LambdaBuilder, DEFAULT_CONTAINER_CMD, DEFAULT_REPO, DEFAULT_REV,
};
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

/// Build the project in a container for deployment to Lambda.
#[derive(Debug, FromArgs)]
pub struct Opt {
    /// lambda-rust repo (default: https://github.com/softprops/lambda-rust)
    #[argh(option, default = "DEFAULT_REPO.into()")]
    repo: String,

    /// branch/tag/commit from which to build (default: master)
    #[argh(option, default = "DEFAULT_REV.into()")]
    rev: String,

    /// container command (default: docker)
    #[argh(option, default = "DEFAULT_CONTAINER_CMD.into()")]
    cmd: String,

    /// path of the project to build
    #[argh(positional, default = "env::current_dir().unwrap()")]
    project: PathBuf,
}

#[throws]
fn main() {
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(log::LevelFilter::Info))?;

    let opt: Opt = argh::from_env();
    let builder = LambdaBuilder {
        repo: opt.repo,
        rev: opt.rev,
        container_cmd: opt.cmd,
        project: opt.project,
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
        let mut usage = Opt::from_args(&["lambda-build"], &["--help"])
            .unwrap_err()
            .output;
        // Remove the "Usage: " prefix which is not in the readme
        usage = usage.replace("Usage: ", "");
        assert!(readme.contains(&usage));
    }
}
