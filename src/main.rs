use anyhow::Error;
use fehler::throws;
use lambda_build::{run, Opt};

#[throws]
fn main() {
    let opt: Opt = argh::from_env();
    run(&opt)?;
}

#[cfg(test)]
mod tests {
    use super::*;
    use argh::FromArgs;

    /// Test that the readme's usage section is up to date
    #[test]
    fn test_readme_usage() {
        let readme = include_str!("../README.md");
        let mut usage = Opt::from_args(&["lambda-build"], &["--help"])
            .unwrap_err()
            .output;
        // Remove the "Usage: " prefix which is not in the readme
        usage = usage.replace("Usage: ", "");
        assert!(readme.contains(&usage));
    }
}
