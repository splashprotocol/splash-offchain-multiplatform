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
COPY --from=builder /app/bloom-cardano-agent/resources/mainnet.deployment.json /app/resources/mainnet.deployment.json
COPY --from=builder /app/bloom-cardano-agent/resources/log4rs.yaml /app/resources/log4rs.yaml
RUN apt-get install -y curl git clang libclang-dev build-essential cmake libcurl4-openssl-dev libssl-dev make
ENTRYPOINT ["/app/bloom-cardano-agent"]
