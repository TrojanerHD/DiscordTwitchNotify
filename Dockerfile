FROM rust:1.84.0

WORKDIR /usr/src/trojaner
COPY ./ ./
RUN cargo build --release

CMD ["./target/release/twitch-notify"]
