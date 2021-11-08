use anyhow::{anyhow, Error};
use argh::FromArgs;
use aws_build_lib::docker_command::command_run::Command;
use aws_build_lib::docker_command::Launcher;
use aws_build_lib::{BuildMode, Builder, DEFAULT_RUST_VERSION};
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

#[throws(String)]
fn parse_command(s: &str) -> Command {
    Command::from_whitespace_separated_str(s)
        .ok_or_else(|| "command is empty".to_string())?
}

#[derive(Debug, FromArgs)]
#[argh(description = "Build the project in a container for deployment to AWS.

mode: al2 or lambda (for Amazon Linux 2 or AWS Lambda, respectively)
project: path of the project to build (default: current directory)
")]
struct Opt {
    /// base container command, e.g. docker or podman, auto-detected by
    /// default
    #[argh(option, from_str_fn(parse_command))]
    container_cmd: Option<Command>,

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

impl Opt {
    #[throws]
    fn launcher(&self) -> Launcher {
        if let Some(cmd) = self.container_cmd.as_ref() {
            Launcher::new(cmd.clone())
        } else {
            Launcher::auto()
                .ok_or_else(|| anyhow!("no container system detected"))?
        }
    }
}

#[throws]
fn main() {
    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(log::LevelFilter::Info))?;

    let opt: Opt = argh::from_env();
    let launcher = opt.launcher()?;

    let builder = Builder {
        rust_version: opt.rust_version,
        mode: opt.mode,
        bin: opt.bin,
        strip: opt.strip,
        launcher,
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
