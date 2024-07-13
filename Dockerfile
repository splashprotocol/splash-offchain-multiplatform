# Build stage
FROM rust:bookworm AS builder
 
WORKDIR /app
COPY . .
RUN apt-get update && apt-get install -y libclang-dev
RUN apt-get update && apt-get install -y libssl-dev
RUN cargo build --package bloom-cardano-agent --release
 
# Final run stage
FROM debian:bookworm-slim AS runner
 
WORKDIR /app
COPY --from=builder /app/target/release/bloom-cardano-agent /app/bloom-cardano-agent
RUN apt-get update && apt-get install -y libclang-dev
RUN apt-get update && apt-get install -y libssl-dev
ENTRYPOINT ["/app/bloom-cardano-agent"]