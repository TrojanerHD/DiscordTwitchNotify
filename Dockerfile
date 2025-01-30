FROM rust:1.84.0

WORKDIR /usr/src/trojaner
COPY Cargo.toml .
COPY Cargo.lock .
COPY src/ src/
RUN cargo build --release

CMD ["./target/release/twitch-notify"]
