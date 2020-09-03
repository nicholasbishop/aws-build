use anyhow::{anyhow, Context, Error};
use fehler::{throw, throws};
use log::info;
use std::path::Path;
use std::process::{Command, ExitStatus};

fn cmd_str(cmd: &Command) -> String {
    format!("{:?}", cmd).replace('"', "")
}

#[throws]
fn run_cmd_no_check(cmd: &mut Command) -> ExitStatus {
    let cmd_str = cmd_str(cmd);
    info!("{}", cmd_str);
    cmd.status().context(format!("failed to run {}", cmd_str))?
}

#[throws]
pub fn run_cmd(cmd: &mut Command) {
    let cmd_str = cmd_str(cmd);
    let status = run_cmd_no_check(cmd)?;
    if !status.success() {
        throw!(anyhow!("command {} failed: {}", cmd_str, status));
    }
}

fn git_cmd_in(repo_path: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo_path);
    cmd
}

/// Clone `repo_url` to `repo_path`.
#[throws]
pub fn git_clone(repo_path: &Path, repo_url: &str) {
    run_cmd(
        Command::new("git")
            .args(&["clone", repo_url])
            .arg(repo_path),
    )?;
}

/// Run `git fetch`.
#[throws]
pub fn git_fetch(repo_path: &Path) {
    run_cmd(git_cmd_in(repo_path).arg("fetch"))?;
}

/// Set the URL of the `origin` remote to `repo_url`.
#[throws]
pub fn git_remote_set_url(repo_path: &Path, repo_url: &str) {
    run_cmd(
        git_cmd_in(repo_path)
            .args(&["remote", "set-url", "origin"])
            .arg(repo_url),
    )?;
}

/// Check out the specified revision.
///
/// First we try checking out `origin/<rev>`. This will work if the
/// rev is a branch, and ensures that we get the latest commit from
/// that branch rather than a local branch that could fall out of
/// date. If that command fails we check out the rev directly, which
/// should work for tags and commit hashes.
#[throws]
pub fn git_checkout(repo_path: &Path, rev: &str) {
    let status = run_cmd_no_check(
        git_cmd_in(&repo_path).args(&["checkout", &format!("origin/{}", rev)]),
    )?;
    if !status.success() {
        run_cmd(git_cmd_in(&repo_path).args(&["checkout", &rev]))?;
    }
}

/// Get the commit hash of the given target.
///
/// Example output: "46794db6816e4a07077cf02711ff1921d50e08d3".
#[throws]
pub fn git_rev_parse(repo: &Path, target: &str) -> String {
    let mut cmd = git_cmd_in(repo);
    cmd.args(&["rev-parse", target]);
    let cmd_str = cmd_str(&cmd);
    let output = cmd.output().context(format!("failed to run {}", cmd_str))?;
    if !output.status.success() {
        throw!(anyhow!("command {} failed: {}", cmd_str, output.status));
    }
    let hash = String::from_utf8(output.stdout)
        .context("failed to convert rev-parse output to a string")?
        .trim()
        .to_string();
    if hash.len() != 40 {
        throw!(anyhow!("invalid commit hash"));
    }
    hash
}
