use anyhow::Error;
use aws_build_lib::docker_command::command_run::Command;
use aws_build_lib::{BuildMode, Builder, Relabel};
use fehler::throws;
use fs_err as fs;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[throws]
fn make_mock_project(root: &Path, name: &str, deps: &[&str]) {
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
    fs::create_dir(root.join("src"))?;
    fs::write(
        root.join("src/main.rs"),
        r#"fn main() {}
            "#,
    )?;
    Command::with_args("cargo", &["generate-lockfile"])
        .set_dir(root)
        .run()?;
}

#[throws]
fn build_and_check(builder: Builder, project_name: &str) {
    let output = builder.run()?;
    let mode_name = match builder.mode {
        BuildMode::AmazonLinux2 => "al2",
        BuildMode::Lambda => "lambda",
    };

    // Symlink points to the real output.
    assert_eq!(fs::canonicalize(&output.symlink)?, output.real);

    // Symlink is at the expected path.
    let expected_symlink_name = format!("latest-{}", mode_name);
    assert_eq!(
        output.symlink,
        builder
            .project_path
            .join("target")
            .join(expected_symlink_name)
    );

    // Real output is in the right directory.
    assert!(output.real.starts_with(
        builder
            .project_path
            .join("target/aws-build")
            .join(mode_name)
    ));

    // Real output's file name has the right form.
    let real_file_name = output.real.file_stem().unwrap();
    let parts = real_file_name
        .to_str()
        .unwrap()
        .split('-')
        .collect::<Vec<_>>();
    dbg!(real_file_name);
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], mode_name);
    assert_eq!(parts[1], project_name);
    assert_eq!(parts[2].len(), 8);
    assert_eq!(parts[3].len(), 16);

    // Real output's extension is correct.
    let expected_extension = match builder.mode {
        BuildMode::AmazonLinux2 => None,
        BuildMode::Lambda => Some(OsStr::new("zip")),
    };
    assert_eq!(output.real.extension(), expected_extension);
}

/// Simple Amazon Linux 2 test.
#[test]
#[throws]
fn test_al2() {
    let root = TempDir::new()?;
    let root = root.path();
    let project_name = "proj";
    make_mock_project(root, project_name, &[])?;
    let builder = Builder {
        mode: BuildMode::AmazonLinux2,
        project_path: root.into(),
        code_root: root.into(),
        relabel: Some(Relabel::Unshared),
        ..Default::default()
    };
    build_and_check(builder, project_name)?;
}

/// Simple Lambda test.
#[test]
#[throws]
fn test_lambda() {
    let root = TempDir::new()?;
    let root = root.path();
    let project_name = "proj";
    make_mock_project(root, project_name, &[])?;
    let builder = Builder {
        mode: BuildMode::Lambda,
        project_path: root.into(),
        code_root: root.into(),
        relabel: Some(Relabel::Unshared),
        ..Default::default()
    };
    build_and_check(builder, project_name)?;
}

/// Test that downloading dependencies works.
///
/// The dependency is arbitrary, just want to check that any dependency
/// works from within the container.
#[test]
#[throws]
fn test_with_deps() {
    let root = TempDir::new()?;
    let root = root.path();
    let project_name = "proj";
    let dep = r#"arrayvec = { version = "0.7.2", default-features = false }"#;
    make_mock_project(root, project_name, &[dep])?;
    let builder = Builder {
        mode: BuildMode::AmazonLinux2,
        project_path: root.into(),
        code_root: root.into(),
        relabel: Some(Relabel::Unshared),
        ..Default::default()
    };
    build_and_check(builder, project_name)?;
}

struct TwoProjects {
    tmp_dir: TempDir,
    proj1: &'static str,
    proj2: &'static str,
}

impl TwoProjects {
    fn proj1_path(&self) -> PathBuf {
        self.root().join(self.proj1)
    }

    fn proj2_path(&self) -> PathBuf {
        self.root().join(self.proj2)
    }

    #[throws]
    fn new() -> TwoProjects {
        let tmp_dir = TempDir::new()?;
        let projects = TwoProjects {
            tmp_dir,
            proj1: "proj1",
            proj2: "proj2",
        };

        fs::create_dir(projects.proj1_path())?;
        fs::create_dir(projects.proj2_path())?;

        make_mock_project(&projects.proj1_path(), projects.proj1, &[])?;

        make_mock_project(
            &projects.proj2_path(),
            projects.proj2,
            &[r#"proj1 = { path = "../proj1" }"#],
        )?;

        projects
    }

    fn root(&self) -> &Path {
        self.tmp_dir.path()
    }
}

/// Test that building a project in a subdirectory of the code root
/// works.
#[test]
#[throws]
fn test_code_root() {
    let projects = TwoProjects::new()?;

    let builder = Builder {
        mode: BuildMode::AmazonLinux2,
        code_root: projects.root().into(),
        project_path: projects.proj2_path(),
        relabel: Some(Relabel::Unshared),
        ..Default::default()
    };
    build_and_check(builder, projects.proj2)?;
}

/// Test that a project path outside the code root fails.
#[test]
#[throws]
fn test_bad_project_path() {
    let projects = TwoProjects::new()?;

    let builder = Builder {
        mode: BuildMode::AmazonLinux2,
        code_root: projects.proj1_path().into(),
        project_path: projects.proj2_path(),
        relabel: Some(Relabel::Unshared),
        ..Default::default()
    };
    assert!(build_and_check(builder, projects.proj2).is_err());
}
