use argh::FromArgs;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{exit, Command, ExitStatus};
use std::{env, fs};

static DEFAULT_REPO: &str = "https://github.com/softprops/lambda-rust";
static DEFAULT_REV: &str = "master";
static DEFAULT_CONTAINER_CMD: &str = "docker";

fn abort(s: &str) -> ! {
    eprintln!("{}", s);
    exit(1);
}

fn cmd_str(cmd: &Command) -> String {
    format!("{:?}", cmd).replace('"', "")
}

fn run_cmd_no_check(cmd: &mut Command) -> ExitStatus {
    let cmd_str = cmd_str(cmd);
    println!("{}", cmd_str);
    let status = match cmd.status() {
        Ok(status) => status,
        Err(err) => {
            abort(&format!("failed to run {}: {}", cmd_str, err));
        }
    };
    status
}

fn run_cmd(cmd: &mut Command) {
    let cmd_str = cmd_str(cmd);
    let status = run_cmd_no_check(cmd);
    if !status.success() {
        abort(&format!("command {} failed: {}", cmd_str, status));
    }
}

fn git_cmd_in(repo_path: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo_path);
    cmd
}

/// Build the project in a container for deployment to Lambda.
#[derive(FromArgs)]
struct Opt {
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

fn main() {
    let opt: Opt = argh::from_env();
    let repo_url = &opt.repo;

    let home: PathBuf = if let Some(home) = env::var_os("HOME") {
        Path::new(&home).into()
    } else {
        abort("HOME is not set");
    };
    let cache: PathBuf = if let Some(cache) = env::var_os("XDG_CACHE_HOME") {
        Path::new(&cache).into()
    } else {
        home.join(".cache")
    };
    let repo_path = cache.join("lambda-build/repo");
    let _ = fs::create_dir_all(&repo_path);

    if !repo_path.join(".git").exists() {
        // Clone the repo if it doesn't exist
        run_cmd(
            Command::new("git")
                .args(&["clone", repo_url])
                .arg(&repo_path),
        );
    } else {
        // Ensure the remote is set correctly
        run_cmd(
            git_cmd_in(&repo_path)
                .args(&["remote", "set-url", "origin"])
                .arg(repo_url),
        );
        // Fetch updates
        run_cmd(git_cmd_in(&repo_path).arg("fetch"));
    };

    // Check out the specified revision. First we try checking out
    // `origin/<rev>`. This will work if the rev is a branch, and
    // ensures that we get the latest commit from that branch rather
    // than a local branch that could fall out of date. If that
    // command fails we check out the rev directly, which should work
    // for tags and commit hashes.
    let status = run_cmd_no_check(
        git_cmd_in(&repo_path)
            .args(&["checkout", &format!("origin/{}", opt.rev)]),
    );
    if !status.success() {
        run_cmd(git_cmd_in(&repo_path).args(&["checkout", &opt.rev]));
    }

    // Build the container
    let image_tag = "rust-lambda-build";
    run_cmd(
        Command::new(&opt.cmd)
            .current_dir(&repo_path)
            .args(&["build", "--tag", image_tag, "."]),
    );

    let volume = |src: &Path, dst: &Path| {
        let mut s = OsString::new();
        s.push(src);
        s.push(":");
        s.push(dst);
        s
    };
    let volume_read_only = |src, dst| {
        let mut s = volume(src, dst);
        s.push(":ro");
        s
    };

    // Ensure that the target directory exists. The output directory
    // ("lambda-target") is mounted to /code/target in the container,
    // but we mount /code from the host read-only, so the target
    // subdirectory needs to already exist. Usually the "target"
    // directory will already exist on the host, but won't if "cargo
    // test" or similar hasn't been run yet.
    let _ = fs::create_dir(opt.project.join("target"));

    // Create the output directory if it doesn't already exist. This
    // ensures it has the right permissions instead of being owned by
    // root.
    let output_dir = opt.project.join("lambda-target");
    let _ = fs::create_dir(&output_dir);

    // Create two cache directories to speed up rebuilds. These are
    // host mounts rather than volumes so that the permissions aren't
    // set to root only.
    let registry_dir = output_dir.join("cargo-registry");
    let _ = fs::create_dir(&registry_dir);
    let git_dir = output_dir.join("cargo-git");
    let _ = fs::create_dir(&git_dir);

    // Run the container
    run_cmd(
        Command::new(&opt.cmd)
            .args(&["run", "--rm", "--init"])
            .arg("-u")
            .arg(format!(
                "{}:{}",
                users::get_current_uid(),
                users::get_current_gid()
            ))
            // Mount the project directory
            .arg("-v")
            .arg(volume_read_only(&opt.project, Path::new("/code")))
            // Mount two Docker volumes to make rebuilds faster
            .arg("-v")
            .arg(volume(&registry_dir, Path::new("/cargo/registry")))
            .arg("-v")
            .arg(volume(&git_dir, Path::new("/cargo/git")))
            // Mount the output target directory
            .arg("-v")
            .arg(volume(&output_dir, Path::new("/code/target")))
            .arg(image_tag),
    );
}
