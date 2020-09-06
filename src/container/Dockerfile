ARG FROM_IMAGE
FROM $FROM_IMAGE

ARG RUST_VERSION
RUN curl -o /rustup.sh --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs
RUN CARGO_HOME=/cargo RUSTUP_HOME=/rustup sh /rustup.sh -y --profile minimal --default-toolchain $RUST_VERSION
ADD build.sh /build.sh
RUN chmod +x /build.sh 
VOLUME ["/code"]
WORKDIR /code

ENTRYPOINT ["/build.sh"]
