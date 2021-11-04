use anyhow::{anyhow, Error};
use argh::FromArgs;
use camino::Utf8PathBuf;
use command_run::Command;
use fehler::throws;
use std::env;

/// Custom tasks.
#[derive(Debug, FromArgs)]
struct Opt {
    #[argh(subcommand)]
    action: Action,
}

#[derive(Debug, FromArgs)]
#[argh(subcommand)]
enum Action {
    DockerTest(DockerTest),
}

/// Test that building with Docker works.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "docker-test")]
struct DockerTest {}

/// Get the absolute path of the repo. Assumes that this executable is
/// located at <repo>/target/<buildmode>/<exename>.
#[throws]
fn get_repo_path() -> Utf8PathBuf {
    let exe = Utf8PathBuf::from_path_buf(env::current_exe()?)
        .map_err(|_| anyhow!("exe path is not utf-8"))?;
    exe.parent()
        .map(|path| path.parent())
        .flatten()
        .map(|path| path.parent())
        .flatten()
        .ok_or_else(|| anyhow!("not enough parents: {}", exe))?
        .into()
}

#[throws]
fn run_docker_test() {
    let repo_dir = get_repo_path()?;

    Command::with_args(
        "cargo",
        &[
            "run",
            "--bin",
            "aws-build",
            "--",
            "al2",
            "--bin",
            "aws-build",
        ],
    )
    .set_dir(&repo_dir)
    .run()?;

    // Check that one output file was created.
    let output =
        glob::glob(repo_dir.join("target/al2/al2-aws-build-*").as_str())?;
    assert_eq!(output.count(), 1);
}

#[throws]
fn main() {
    let opt: Opt = argh::from_env();

    match opt.action {
        Action::DockerTest(_) => run_docker_test()?,
    }
}
