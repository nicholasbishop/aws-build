# lambda-build

Build a Rust project in a container for deployment to AWS Lambda.

This project is a thin wrapper around the excellent
[lambda-rust](https://github.com/softprops/lambda-rust) project, which
provides "a faithful reproduction of the actual AWS [...] Lambda
runtime environment" with the stable Rust toolchain installed.

The lambda-build executable expands on lambda-rust in two ways:
1. It downloads the lambda-rust repo and builds a specific branch,
   tag, or commit instead of using a build from Docker hub. This is
   useful because the lambda-rust repo is sometimes updated without a
   new tag being pushed to Docker hub.
2. It builds and runs the lambda-rust container with all the necessary
   options. There are a number of volumes that need to get mounted in
   the right place for caching to work.
   
## Installation

```
cargo install lambda-build
```

## Usage

In the common case you should be able to just run `lambda-build` in
the directory of the project you want to build. You can also pass an
explicit directory to build. By default the master branch of
`lambda-rust` is used; a different one can be set with `--rev`.

The lambda package will be output to
`lambda-target/lambda/release/lambda-build.zip`.

```
lambda-build [<project>] [--repo <repo>] [--rev <rev>] [--cmd <cmd>]

Build the project in a container for deployment to Lambda.

Options:
  --repo            lambda-rust repo (default:
                    https://github.com/softprops/lambda-rust)
  --rev             branch/tag/commit from which to build (default: master)
  --cmd             container command (default: docker)
  --help            display usage information
```

## Related projects

[cargo-aws-lambda](https://github.com/vvilhonen/cargo-aws-lambda):
"dependency free cargo subcommand for cross-compiling, packaging and
deploying code quickly to AWS Lambda"
