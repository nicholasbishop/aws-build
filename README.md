# aws-build

**This tool is no longer under active development. If you are interested in taking over or repurposing the name on crates.io, feel free to contact me: nbishop@nbishop.net**

[![crates.io](https://img.shields.io/crates/v/aws-build.svg)](https://crates.io/crates/aws-build)
[![Documentation](https://docs.rs/aws-build-lib/badge.svg)](https://docs.rs/aws-build-lib)

Build a Rust project in a container for deployment to either an
instance running AWS Linux 2 or AWS Lambda.

Both a [library](https://crates.io/crates/aws-build-lib) and an
[executable](https://crates.io/crates/aws-build) are provided. The
executable is a very thin wrapper around the library.

This crate only handles building the project locally. It does not
interact with any AWS services.

## Executable

Install with:

```
cargo install aws-build
```

In the common case you should be able to just run `aws-build al2` or
`aws-build lambda` in the directory of the project you want to
build.

On successful completion, the output file (either a standalone executable
for Amazon Linux 2 or a zip file containing a "bootstrap" executable
for AWS Lambda) is written to a subdirectory of the `target`
directory. There is also a `target/latest-al2` or
`target/latest-lambda` symlink that points to the output file.

```
aws-build <mode> [<project>] [--container-cmd <container-cmd>] [--rust-version <rust-version>] [--strip] [--bin <bin>] [--package <package...>] [--code-root <code-root>]

Build the project in a container for deployment to AWS.

mode: al2 or lambda (for Amazon Linux 2 or AWS Lambda, respectively)
project: path of the project to build (default: current directory)

Options:
  --container-cmd   base container command, e.g. docker or podman, auto-detected
                    by default
  --rust-version    rust version (default: latest stable)
  --strip           strip debug symbols
  --bin             name of the binary target to build (required if there is
                    more than one binary target)
  --package         yum devel package to install in build container
  --code-root       root directory to mount into the container, must contain the
                    project path (default: project path)
  --help            display usage information
```

## Related projects

- [amazon-linux](https://hub.docker.com/_/amazonlinux): Docker image
  replicating the Aamazon Linux environment.
- [cargo-aws-lambda](https://github.com/vvilhonen/cargo-aws-lambda):
  "dependency free cargo subcommand for cross-compiling, packaging and
  deploying code quickly to AWS Lambda"
- [docker-lambda](https://github.com/lambci/docker-lambda): "A
  sandboxed local environment that replicates the live AWS Lambda
  environment almost identically"
- [lambda-rust](https://github.com/softprops/lambda-rust): "a faithful
  reproduction of the actual AWS [...] Lambda runtime environment" with
  the stable Rust toolchain installed
