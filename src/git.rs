use crate::command::{cmd_str, run_cmd, run_cmd_no_check};
use anyhow::{anyhow, Context, Error};
use fehler::{throw, throws};
use std::path::PathBuf;
use std::process::Command;

pub struct Repo {
    pub path: PathBuf,
}

impl Repo {
    pub fn new(path: PathBuf) -> Repo {
        Repo { path }
    }

    fn git_cmd_in(&self) -> Command {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.path);
        cmd
    }

    /// Clone `repo_url`.
    #[throws]
    pub fn clone(&self, repo_url: &str) {
        run_cmd(
            Command::new("git")
                .args(&["clone", repo_url])
                .arg(&self.path),
        )?;
    }

    /// Run `git fetch`.
    #[throws]
    pub fn fetch(&self) {
        run_cmd(self.git_cmd_in().arg("fetch"))?;
    }

    /// Set the URL of the `origin` remote to `repo_url`.
    #[throws]
    pub fn remote_set_url(&self, repo_url: &str) {
        run_cmd(
            self.git_cmd_in()
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
    pub fn checkout(&self, rev: &str) {
        let status = run_cmd_no_check(
            self.git_cmd_in()
                .args(&["checkout", &format!("origin/{}", rev)]),
        )?;
        if !status.success() {
            run_cmd(self.git_cmd_in().args(&["checkout", &rev]))?;
        }
    }

    /// Get the commit hash of the given target.
    ///
    /// Example output: "46794db6816e4a07077cf02711ff1921d50e08d3".
    #[throws]
    pub fn rev_parse(&self, target: &str) -> String {
        let mut cmd = self.git_cmd_in();
        cmd.args(&["rev-parse", target]);
        let cmd_str = cmd_str(&cmd);
        let output =
            cmd.output().context(format!("failed to run {}", cmd_str))?;
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
}
