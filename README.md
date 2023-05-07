# grpc-web

[![Crates.io][crates-badge]][crates-url]
[![License][license-badge]][license-url]
[![Build Status][actions-badge]][actions-url]

[crates-badge]: https://img.shields.io/crates/v/grpc-web.svg
[crates-url]: https://crates.io/crates/grpc-web
[license-badge]: https://img.shields.io/crates/l/grpc-web.svg
[license-url]: #license
[actions-badge]: https://github.com/Millione/grpc-web/actions/workflows/ci.yaml/badge.svg
[actions-url]: https://github.com/Millione/grpc-web/actions

Enables volo-grpc servers to handle requests from `grpc-web` clients directly, without the need of an external proxy.

## Usage

Add this to your `Cargo.toml`:

```toml
[build-dependencies]
grpc-web = "0.1"
```

## Example

The easiest way to get started, is to call the function with your volo-grpc service and allow the volo-grpc server to accept HTTP/1.1 requests:

```rust
#[tokio::main]
async fn main() {
    let addr: SocketAddr = "[::]:8080".parse().unwrap();
    let addr = volo::net::Address::from(addr);

    Server::new()
        .accept_http1(true)
        .layer_outer(WebLayer::new(Cors::new(Config::default())))
        .add_service(ServiceBuilder::new(GreeterServer::new(S)).build())
        .run(address)
        .await
        .unwrap()
}
```

See [the examples folder][example] for a server and client example.

[example]: https://github.com/Millione/grpc-web/tree/main/examples/src

## License

Dual-licensed under the MIT license and the Apache License (Version 2.0).

See [LICENSE-MIT](https://github.com/Millione/grpc-web/blob/main/LICENSE-MIT) and [LICENSE-APACHE](https://github.com/Millione/grpc-web/blob/main/LICENSE-APACHE) for details.
