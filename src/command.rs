use anyhow::{anyhow, Context, Error};
use fehler::{throw, throws};
use log::info;
use std::process::{Command, ExitStatus};

pub fn cmd_str(cmd: &Command) -> String {
    format!("{:?}", cmd).replace('"', "")
}

#[throws]
pub fn run_cmd_no_check(cmd: &mut Command) -> ExitStatus {
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
