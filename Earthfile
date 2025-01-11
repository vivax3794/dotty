VERSION 0.8
IMPORT github.com/earthly/lib/rust:3.0.3 AS rust

env:
    FROM rustlang/rust:nightly-slim
    WORKDIR /app
    ENV CARGO_TERM_COLOR=always
    DO rust+INIT

build:
    FROM +env
    COPY Cargo.toml Cargo.lock .
    COPY src ./src
    DO rust+CARGO --args="build" --output="debug/dotty"
    SAVE ARTIFACT ./target/debug/dotty dotty AS LOCAL "./dotty"
