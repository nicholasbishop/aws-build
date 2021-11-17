//! Run tests on the aws-build binary. This is done via xtask rather
//! than normal tests so that we can test the end-user binary, not just
//! the library.

use anyhow::{anyhow, Error};
use argh::FromArgs;
use camino::{Utf8Path, Utf8PathBuf};
use command_run::Command;
use fehler::throws;
use fs_err as fs;
use std::env;
use std::ffi::OsStr;

/// Custom tasks.
#[derive(Debug, FromArgs)]
struct Opt {
    #[argh(subcommand)]
    action: Action,
}

#[derive(Debug, FromArgs)]
#[argh(subcommand)]
enum Action {
    RunContainerTests(RunContainerTests),
}

/// Run "live" tests using docker or podman.
#[derive(Debug, FromArgs)]
#[argh(subcommand, name = "run-container-tests")]
struct RunContainerTests {
    /// delete the cache directory before running the tests
    #[argh(switch)]
    clean: bool,

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
fn make_mock_project(root: &Utf8Path, name: &str, deps: &[&str]) {
    fs::create_dir_all(root)?;

    let toml = format!(
        r#"
        [package]
        name = "{}"
        version = "0.0.0"
        [dependencies]
        {}
        "#,
        name,
        deps.join("\n"),
    );

    fs::write(root.join("Cargo.toml"), toml)?;
    fs::create_dir_all(root.join("src"))?;
    fs::write(
        root.join("src/main.rs"),
        r#"fn main() {}
            "#,
    )?;
    Command::with_args("cargo", &["generate-lockfile"])
        .set_dir(root)
        .run()?;
}

enum BuildMode {
    Al2,
    Lambda,
}

impl BuildMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Al2 => "al2",
            Self::Lambda => "lambda",
        }
    }

    fn extension(&self) -> Option<&'static OsStr> {
        match self {
            Self::Al2 => None,
            Self::Lambda => Some(OsStr::new("zip")),
        }
    }
}

struct Checker<'a> {
    mode: BuildMode,
    project_name: &'a str,
    project_path: Utf8PathBuf,
    code_root: Option<&'a Utf8Path>,
}

impl<'a> Checker<'a> {
    /// Build the project and return the output symlink path.
    #[throws]
    fn build(&self, test_input: &TestInput) -> Utf8PathBuf {
        let mut cmd =
            Command::with_args("cargo", &["run", "--bin", "aws-build", "--"]);
        if let Some(code_root) = self.code_root {
            cmd.add_args(&["--code-root", code_root.as_str()]);
        }
        cmd.add_args(&[self.mode.as_str(), self.project_path.as_str()]);
        cmd.set_dir(test_input.repo_dir);
        cmd.enable_capture();
        cmd.combine_output();
        cmd.log_output_on_error = true;

        if let Some(container_cmd) = &test_input.container_cmd {
            cmd.add_args(&["--container-cmd", container_cmd]);
        }

        let output = cmd.run()?;
        let stdout = output.stdout_string_lossy();
        let symlink_path = stdout
            .lines()
            .find_map(|line| line.strip_prefix("symlink: "))
            .ok_or_else(|| anyhow!("symlink not found in output"))?;
        Utf8PathBuf::from(symlink_path)
    }

    #[throws]
    fn build_and_check(&self, test_input: &TestInput) {
        let symlink_path = self.build(test_input)?;
        let real_output_path = fs::canonicalize(&symlink_path)?;

        let target_dir = self.project_path.join("target");

        // Symlink is at the expected path.
        let expected_symlink_name = format!("latest-{}", self.mode.as_str());
        assert_eq!(symlink_path, target_dir.join(expected_symlink_name));

        // Real output is in the right directory.
        assert!(real_output_path.starts_with(
            target_dir.join("aws-build").join(self.mode.as_str())
        ));

        // Real output's file name has the right form.
        let real_file_name = real_output_path.file_stem().unwrap();
        let parts = real_file_name
            .to_str()
            .unwrap()
            .split('-')
            .collect::<Vec<_>>();
        dbg!(real_file_name);
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0], self.mode.as_str());
        assert_eq!(parts[1], self.project_name);
        assert_eq!(parts[2].len(), 8);
        assert_eq!(parts[3].len(), 16);

        // Real output's extension is correct.
        assert_eq!(real_output_path.extension(), self.mode.extension());
    }
}

struct TestInput<'a> {
    container_cmd: Option<&'a str>,
    repo_dir: &'a Utf8Path,
    // TODO
    base_test_dir: &'a Utf8Path,
    test_dir: Utf8PathBuf,
}

/// Simple Amazon Linux 2 test.
#[throws]
fn test_al2(test_input: &TestInput) {
    let project_name = "proj";
    make_mock_project(&test_input.test_dir, project_name, &[])?;
    Checker {
        mode: BuildMode::Al2,
        project_name,
        project_path: test_input.test_dir.clone(),
        code_root: None,
    }
    .build_and_check(test_input)?;
}

/// Simple Lambda test.
#[throws]
fn test_lambda(test_input: &TestInput) {
    let project_name = "proj";
    make_mock_project(&test_input.test_dir, project_name, &[])?;
    Checker {
        mode: BuildMode::Lambda,
        project_name,
        project_path: test_input.test_dir.clone(),
        code_root: None,
    }
    .build_and_check(test_input)?;
}

/// Test that downloading dependencies works.
///
/// The dependency is arbitrary, just want to check that any dependency
/// works from within the container.
#[throws]
fn test_deps(test_input: &TestInput) {
    let project_name = "proj";
    let dep = r#"arrayvec = { version = "0.7.2", default-features = false }"#;
    make_mock_project(&test_input.test_dir, project_name, &[dep])?;
    Checker {
        mode: BuildMode::Al2,
        project_path: test_input.test_dir.clone(),
        project_name,
        code_root: None,
    }
    .build_and_check(test_input)?;
}

struct TwoProjects {
    proj1: &'static str,
    proj2: &'static str,
    proj1_path: Utf8PathBuf,
    proj2_path: Utf8PathBuf,
}

impl TwoProjects {
    #[throws]
    fn new(test_dir: &Utf8Path) -> TwoProjects {
        let proj1 = "proj1";
        let proj2 = "proj2";
        let projects = TwoProjects {
            proj1,
            proj2,
            proj1_path: test_dir.join(proj1),
            proj2_path: test_dir.join(proj2),
        };

        fs::create_dir_all(&projects.proj1_path)?;
        fs::create_dir_all(&projects.proj2_path)?;

        make_mock_project(&projects.proj1_path, projects.proj1, &[])?;

        make_mock_project(
            &projects.proj2_path,
            projects.proj2,
            &[r#"proj1 = { path = "../proj1" }"#],
        )?;

        projects
    }
}

/// Test that building a project in a subdirectory of the code root
/// works.
#[throws]
fn test_code_root(test_input: &TestInput) {
    let projects = TwoProjects::new(&test_input.test_dir)?;

    Checker {
        mode: BuildMode::Al2,
        code_root: Some(&test_input.test_dir),
        project_name: projects.proj2,
        project_path: projects.proj2_path,
    }
    .build_and_check(test_input)?;
}

/// Test that a project path outside the code root fails.
#[throws]
fn test_bad_project_path(test_input: &TestInput) {
    let projects = TwoProjects::new(&test_input.test_dir)?;

    let checker = Checker {
        mode: BuildMode::Al2,
        code_root: Some(&projects.proj1_path),
        project_name: projects.proj2,
        project_path: projects.proj2_path,
    };
    assert!(checker.build_and_check(test_input).is_err());
}

type TestFn = fn(&TestInput) -> Result<(), Error>;

#[throws]
fn run_build_test(args: RunContainerTests) {
    let repo_dir = get_repo_path()?;

    let mut test_input = TestInput {
        container_cmd: args.container_cmd.as_deref(),
        repo_dir: &repo_dir,
        base_test_dir: &repo_dir.join("container_tests"),
        test_dir: Default::default(),
    };
    if args.clean {
        println!("cleaning {}", test_input.base_test_dir);
        fs::remove_dir_all(test_input.base_test_dir)?;
    }
    fs::create_dir_all(test_input.base_test_dir)?;
    let tf = |f: TestFn, s: &'static str| (f, s);
    let test_funcs = &[
        tf(test_al2, "test_al2"),
        tf(test_lambda, "test_lambda"),
        tf(test_deps, "test_deps"),
        tf(test_code_root, "test_code_root"),
        tf(test_bad_project_path, "test_bad_project_path"),
    ];
    // TODO: run in parallel? If not, just call them directly
    for (func, test_name) in test_funcs {
        test_input.test_dir = test_input.base_test_dir.join(test_name);
        func(&test_input)?;
    }

    println!("success");
}

#[throws]
fn main() {
    let opt: Opt = argh::from_env();

    match opt.action {
        Action::RunContainerTests(args) => run_build_test(args)?,
    }
}
