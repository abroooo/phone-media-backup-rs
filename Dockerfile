# Build stage
FROM rust:1.95-trixie AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --locked

# Runtime stage
FROM debian:trixie-slim
# Install any runtime dependencies your app needs
RUN apt-get update && apt-get install -y openssh-client ca-certificates sshfs && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/p2r /usr/local/bin/p2r
RUN mkdir /root/sshmount
ENTRYPOINT ["p2r"]
