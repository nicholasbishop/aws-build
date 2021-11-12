# The FROM_IMAGE arg can be set to either an Amazon Linux 2 image or a
# Lambda image
ARG FROM_IMAGE
FROM $FROM_IMAGE

# This is already in the Lambda image but not in the AL2 image
RUN yum install -y gcc

# Install Rust
ARG RUST_VERSION
RUN curl -o /rustup.sh --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs
RUN CARGO_HOME=/cargo RUSTUP_HOME=/rustup sh /rustup.sh -y --profile minimal --default-toolchain $RUST_VERSION

ARG DEV_PKGS
RUN if [[ ! -z "$DEV_PKGS" ]] ; then yum install -y $DEV_PKGS ; fi

# Add the build script
ADD build.sh /build.sh
RUN chmod +x /build.sh

VOLUME ["/code"]

# Change to the project directory.
ARG PROJECT_PATH
WORKDIR /code/"$PROJECT_PATH"

ENTRYPOINT ["/build.sh"]
