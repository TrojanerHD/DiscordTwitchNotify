FROM rust:1.84.0

WORKDIR /usr/src/trojaner
COPY Cargo.toml .
COPY Cargo.lock .
COPY src/ src/
RUN cargo build --release

RUN cp target/release/twitch-notify twitch-notify
RUN rm -r src Cargo.toml Cargo.lock target

CMD ["./twitch-notify"]
