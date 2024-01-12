# Client-Server

Implementation of a TCP server and client.

## [cli-ser](./cli-ser)

A library that facilitates the foundation for server-client communication.
It defines basic structures (e.g., Image) and message types.

## [client](./client)

An implementation of a TCP client built upon the `cli-ser` crate.
It initially connects to the desired address specified through command-line arguments.
The client executes commands given in the terminal e.g. specified message is constructed and send to the server.
It also acts accordingly to the messages it receives, texts are displayed to the screen, files are saved.

## [server](./server)

Implementation of a TCP server built upon the `cli-ser` crate.
It listens at a specified address provided through command-line arguments,
and maintains user information and message history in a database
The server mandates authentication for new connections.
Currently it supports broadcasting client messages.
Additionally, it can send error messages to the appropriate connections.

### !!!Database Setup!!!

`server` needs a running database and its URL specified, see `server`s documentation.

## Requirements

1. [Network I/O](https://robot-dreams-rust.mag.wiki/9-network-io/index.html#homework)
2. [Rust Ecosystem](https://robot-dreams-rust.mag.wiki/11-rust-ecosystem/index.html#homework)
3. [Error Handling](https://robot-dreams-rust.mag.wiki/13-error-handling-custom-types/index.html#homework)
4. [Async Programming](https://robot-dreams-rust.mag.wiki/15-async-programming-tokio/index.html#homework)
5. [Testing and Documentation](https://robot-dreams-rust.mag.wiki/16-testing-and-documentation/index.html#homework)

## [Example images](./example-images)

Example images were taken from these sites:

* <https://rustacean.net/assets/rustacean-orig-noshadow.png>
* <https://github.com/rust-community/resources/blob/gh-pages/sticker/rust/examples/hexagon.jpeg>
