# Client-Server

The following crates implement the requirements for these homework assignments:

1. [Network I/O](https://robot-dreams-rust.mag.wiki/9-network-io/index.html#homework)
2. [Rust Ecosystem](https://robot-dreams-rust.mag.wiki/11-rust-ecosystem/index.html#homework)
3. [Error Handling](https://robot-dreams-rust.mag.wiki/13-error-handling-custom-types/index.html#homework)
4. [Async Programming](https://robot-dreams-rust.mag.wiki/15-async-programming-tokio/index.html#homework)

## [client](./client)

A binary crate that functions as a TCP client.
Initially, it connects to the desired address (specified by command-line arguments).
Then it facilitates the sending and receiving of messages to and from the server.
The `client` depends on the `cli-ser` crate.

## [server](./server)

A binary crate that acts as a server.
It listens at a specified address (provided through command-line arguments).
Received messages are broadcasted to all other connected clients.
The `server` also depends on the `cli-ser` crate.

## [cli-ser](./cli-ser)

Library crate that proves to be handy when implementing either a client or server.

## [Example images](./example-images)

Example images were taken from these sites:

* <https://rustacean.net/assets/rustacean-orig-noshadow.png>
* <https://github.com/rust-community/resources/blob/gh-pages/sticker/rust/examples/hexagon.jpeg>
