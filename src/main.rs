use argh::FromArgs;
use std::process::Command;

static DEFAULT_REPO: &str = "https://github.com/softprops/lambda-rust";
static DEFAULT_REV: &str = "master";
static DEFAULT_CONTAINER_CMD: &str = "docker";

fn cmd_str(cmd: &Command) -> String {
    format!("{:?}", cmd).replace('"', "")
}

fn run_cmd(cmd: &mut Command) {
    let cmd_str = cmd_str(cmd);
    println!("{}", cmd_str);
    let status = cmd.status().unwrap();
    if !status.success() {
        panic!("command {} failed: {}", cmd_str, status);
    }
}

/// Build the project in a container for deployment to Lambda.
#[derive(FromArgs)]
struct Opt {
    /// lambda-rust repo (default: https://github.com/softprops/lambda-rust)
    #[argh(option, default="DEFAULT_REPO.into()")]
    repo: String,

    /// branch/tag/commit from which to build (default: master)
    #[argh(option, default="DEFAULT_REV.into()")]
    rev: String,

    /// container command (default: docker)
    #[argh(option, default="DEFAULT_CONTAINER_CMD.into()")]
    cmd: String,
}

fn main() {
    let opt: Opt = argh::from_env();

    println!("Hello, world!");
}
