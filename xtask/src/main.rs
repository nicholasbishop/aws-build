//! Run tests on the aws-build binary. This is done via xtask rather
//! than normal tests so that we can test the end-user binary, not just
//! the library.

use anyhow::{anyhow, Error};
use argh::FromArgs;
use command_run::Command;
use fehler::throws;
use fs_err as fs;
use rayon::prelude::*;
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

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

    /// run a single test with the given name
    #[argh(option)]
    name: Option<String>,
}

/// Get the absolute path of the repo. Assumes that this executable is
/// located at <repo>/target/<buildmode>/<exename>.
#[throws]
fn get_repo_path() -> PathBuf {
    let exe = env::current_exe()?;
    exe.parent()
        .map(|path| path.parent())
        .flatten()
        .map(|path| path.parent())
        .flatten()
        .ok_or_else(|| anyhow!("not enough parents: {}", exe.display()))?
        .into()
}

#[throws]
fn make_mock_project(root: &Path, name: &str, deps: &[&str]) {
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
    project_path: PathBuf,
    code_root: Option<&'a Path>,
}

impl<'a> Checker<'a> {
    /// Build the project and return the output symlink path.
    #[throws]
    fn build(&self, test_input: &TestInput) -> PathBuf {
        let mut cmd =
            Command::with_args("cargo", &["run", "--bin", "aws-build", "--"]);
        if let Some(code_root) = self.code_root {
            cmd.add_arg("--code-root");
            cmd.add_arg(code_root);
        }
        cmd.add_arg(self.mode.as_str());
        cmd.add_arg(&self.project_path);
        cmd.set_dir(&test_input.repo_dir);
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
        PathBuf::from(symlink_path)
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

struct TestInput {
    container_cmd: Option<String>,
    repo_dir: PathBuf,
    test_dir: PathBuf,
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
    proj1_path: PathBuf,
    proj2_path: PathBuf,
}

impl TwoProjects {
    #[throws]
    fn new(test_dir: &Path) -> TwoProjects {
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

const TEST_FUNCS: &[(TestFn, &str)] = &[
    (test_al2, "test_al2"),
    (test_lambda, "test_lambda"),
    (test_deps, "test_deps"),
    (test_code_root, "test_code_root"),
    (test_bad_project_path, "test_bad_project_path"),
];

#[throws]
fn run_one_test(args: &RunContainerTests, name: &str) {
    let mut test_input = TestInput {
        container_cmd: args.container_cmd.clone(),
        repo_dir: get_repo_path()?,
        test_dir: Default::default(),
    };
    let base_test_dir = test_input.repo_dir.join("container_tests");

    let (func, test_name) = TEST_FUNCS
        .iter()
        .find(|(_, test_name)| *test_name == name)
        .ok_or_else(|| anyhow!("test '{}' not found", name))?;
    test_input.test_dir = base_test_dir.join(test_name);
    func(&test_input)?;

    println!("success");
}

#[throws]
fn run_all_tests(args: &RunContainerTests) {
    let exe = env::current_exe()?;

    let failures: Vec<_> = TEST_FUNCS
        .par_iter()
        .filter_map(|(_func, test_name)| {
            let mut cmd = Command::with_args(
                exe.clone(),
                &["run-container-tests", "--name", test_name],
            );
            if let Some(container_cmd) = &args.container_cmd {
                cmd.add_args(&["--container-cmd", container_cmd]);
            }
            cmd.combine_output = true;
            cmd.capture = true;
            cmd.check = false;

            let output = cmd.run().expect("failed to run command");
            if output.status.success() {
                None
            } else {
                Some((test_name, output.stdout_string_lossy().to_string()))
            }
        })
        .collect();

    for (test_name, output) in &failures {
        println!("{} failed: {}\n-----\n", test_name, output);
    }

    if !failures.is_empty() {
        panic!("{} test(s) failed", failures.len());
    }
}

#[throws]
fn run_build_test(args: &RunContainerTests) {
    let test_input = TestInput {
        container_cmd: args.container_cmd.clone(),
        repo_dir: get_repo_path()?,
        test_dir: Default::default(),
    };
    let base_test_dir = test_input.repo_dir.join("container_tests");

    if args.clean {
        println!("cleaning {}", base_test_dir.display());
        fs::remove_dir_all(&base_test_dir)?;
    }

    fs::create_dir_all(&base_test_dir)?;

    if let Some(name) = &args.name {
        run_one_test(args, name)?;
    } else {
        run_all_tests(args)?;
    }

    println!("success");
}

#[throws]
fn main() {
    let opt: Opt = argh::from_env();

    match opt.action {
        Action::RunContainerTests(args) => run_build_test(&args)?,
    }
}
