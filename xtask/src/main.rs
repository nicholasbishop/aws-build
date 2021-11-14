use anyhow::{anyhow, Error};
use argh::FromArgs;
use camino::Utf8PathBuf;
use command_run::Command;
use fehler::throws;
use fs_err as fs;
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
    Test(Test),
}

/// Run "live" tests using docker or podman.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "test")]
struct Test {
    /// base container command, e.g. docker or podman, auto-detected by
    /// default
    #[argh(option)]
    container_cmd: Option<String>,
}

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
fn run_build_test(args: Test) {
    let repo_dir = get_repo_path()?;
    let target_dir = repo_dir.join("target");
    let symlink = target_dir.join("latest-al2");

    // Delete the symlink if it already exists.
    if symlink.exists() {
        println!("deleting {}", symlink);
        fs::remove_file(&symlink)?;
    }

    let mut cmd = Command::with_args(
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
    );
    cmd.set_dir(&repo_dir);

    if let Some(container_cmd) = &args.container_cmd {
        cmd.add_args(&["--container-cmd", container_cmd]);
    }

    cmd.run()?;

    println!("symlink: {}", symlink);

    // Check that the symlink was created.
    assert!(symlink.exists());

    // Check that the symlink's target exists.
    let symlink_target = fs::canonicalize(&symlink)?;
    println!("symlink target: {}", symlink_target.display());
    assert!(symlink_target != symlink);
    assert!(symlink_target.exists());

    println!("success");
}

#[throws]
fn main() {
    let opt: Opt = argh::from_env();

    match opt.action {
        Action::Test(args) => run_build_test(args)?,
    }
}
