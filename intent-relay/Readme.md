# Splash Intent Relay

The Splash Intent Relay is a service designed to facilitate communication between clients and execution engines. It acts
as a middleware that accepts intent messages from clients, processes them as needed, and then broadcasts these intents
to the appropriate execution engines for handling. This enables seamless integration and interaction between various
components in the system, ensuring that client requests are efficiently routed to the relevant handlers for execution.

### Run from sources
`cargo run --bin intent-relay --release -- --config-path <config_path> --log4rs-path <log4rs_path> --host <host> --port <port>`

# Running tests and building the project

To run the tests and build the project, you can use the following commands:

### Run tests

```shell
cargo test
```

This will execute all tests in the project.

### Build the project

```shell
cargo build --release
```

This will build the project in release mode, generating an optimized binary.

Make sure `cargo` is installed and configured correctly before running these commands.