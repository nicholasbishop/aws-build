use anyhow::Error;
use aws_build_lib::docker_command::command_run::Command;
use aws_build_lib::{BuildMode, Builder, Relabel};
use fehler::throws;
use fs_err as fs;
use std::ffi::OsStr;
use std::path::Path;
use tempfile::TempDir;

#[throws]
fn make_mock_project(root: &Path, name: &str) {
    let toml = format!(
        r#"
        [package]
        name = "{}"
        version = "0.0.0"
        "#,
        name
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
        builder.project.join("target").join(expected_symlink_name)
    );

    // Real output is in the right directory.
    assert!(output
        .real
        .starts_with(builder.project.join("target/aws-build").join(mode_name)));

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

#[test]
#[throws]
fn test_al2() {
    let root = TempDir::new()?;
    let root = root.path();
    let project_name = "proj";
    make_mock_project(root, project_name)?;
    let builder = Builder {
        mode: BuildMode::AmazonLinux2,
        project: root.into(),
        relabel: Some(Relabel::Unshared),
        ..Default::default()
    };
    build_and_check(builder, project_name)?;
}

#[test]
#[throws]
fn test_lambda() {
    let root = TempDir::new()?;
    let root = root.path();
    let project_name = "proj";
    make_mock_project(root, project_name)?;
    let builder = Builder {
        mode: BuildMode::Lambda,
        project: root.into(),
        relabel: Some(Relabel::Unshared),
        ..Default::default()
    };
    build_and_check(builder, project_name)?;
}
