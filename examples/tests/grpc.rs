use std::{future::Future, net::SocketAddr};

use examples::{
    volo_gen::proto_gen::example::{
        ExampleClient, ExampleClientBuilder, ExampleServer, Input, Output,
    },
    S,
};
use grpc_web::{Cors, WebLayer};
use tokio::{join, time::Duration, try_join};
use tokio_stream::{self as stream, Stream, StreamExt};
use volo::net::Address;
use volo_grpc::{
    server::{Server, ServiceBuilder},
    BoxError, Response, Status,
};

#[tokio::test]
async fn smoke_unary() {
    let (c1, c2, c3, c4) = spawn().await.expect("clients");

    let (r1, r2, r3, r4) = try_join!(
        c1.unary_call(input()),
        c2.unary_call(input()),
        c3.unary_call(input()),
        c4.unary_call(input()),
    )
    .expect("responses");

    assert!(meta(&r1) == meta(&r2) && meta(&r2) == meta(&r3) && meta(&r3) == meta(&r4));
    assert!(data(&r1) == data(&r2) && data(&r2) == data(&r3) && data(&r3) == data(&r4));
}

#[tokio::test]
async fn smoke_client_stream() {
    let (c1, c2, c3, c4) = spawn().await.expect("clients");

    let input_stream = || stream::iter(vec![input(), input()]);

    let (r1, r2, r3, r4) = try_join!(
        c1.client_stream(input_stream()),
        c2.client_stream(input_stream()),
        c3.client_stream(input_stream()),
        c4.client_stream(input_stream()),
    )
    .expect("responses");

    assert!(meta(&r1) == meta(&r2) && meta(&r2) == meta(&r3) && meta(&r3) == meta(&r4));
    assert!(data(&r1) == data(&r2) && data(&r2) == data(&r3) && data(&r3) == data(&r4));
}

#[tokio::test]
async fn smoke_server_stream() {
    let (c1, c2, c3, c4) = spawn().await.expect("clients");

    let (r1, r2, r3, r4) = try_join!(
        c1.server_stream(input()),
        c2.server_stream(input()),
        c3.server_stream(input()),
        c4.server_stream(input()),
    )
    .expect("responses");

    assert!(meta(&r1) == meta(&r2) && meta(&r2) == meta(&r3) && meta(&r3) == meta(&r4));

    let r1 = stream(r1).await;
    let r2 = stream(r2).await;
    let r3 = stream(r3).await;
    let r4 = stream(r4).await;

    assert!(r1 == r2 && r2 == r3 && r3 == r4);
}
#[tokio::test]
async fn smoke_error() {
    let (c1, c2, c3, c4) = spawn().await.expect("clients");

    let boom = Input {
        id: 1,
        desc: "boom".into(),
    };

    let (r1, r2, r3, r4) = join!(
        c1.unary_call(boom.clone()),
        c2.unary_call(boom.clone()),
        c3.unary_call(boom.clone()),
        c4.unary_call(boom.clone()),
    );

    let s1 = r1.unwrap_err();
    let s2 = r2.unwrap_err();
    let s3 = r3.unwrap_err();
    let s4 = r4.unwrap_err();

    assert!(status(&s1) == status(&s2) && status(&s2) == status(&s3) && status(&s3) == status(&s4))
}

async fn grpc(accept_h1: bool) -> (impl Future<Output = Result<(), BoxError>>, SocketAddr) {
    let addr: SocketAddr = "[::]:8081".parse().unwrap();
    let address = Address::from(addr);

    let fut = Server::new()
        .accept_http1(accept_h1)
        .add_service(ServiceBuilder::new(ExampleServer::new(S)).build())
        .run(address);

    (fut, addr)
}

async fn grpc_web(accept_h1: bool) -> (impl Future<Output = Result<(), BoxError>>, SocketAddr) {
    let addr: SocketAddr = "[::]:8081".parse().unwrap();
    let address = Address::from(addr);

    let config = grpc_web::Config::default().allow_origins(vec!["http://foo.com"]);

    let fut = Server::new()
        .accept_http1(accept_h1)
        .layer_outer(WebLayer::new(Cors::new(config)))
        .add_service(ServiceBuilder::new(ExampleServer::new(S)).build())
        .run(address);

    (fut, addr)
}

async fn spawn() -> Result<(ExampleClient, ExampleClient, ExampleClient, ExampleClient), Status> {
    let ((s1, u1), (s2, u2), (s3, u3), (s4, u4)) =
        join!(grpc(true), grpc(false), grpc_web(true), grpc_web(false));

    tokio::spawn(async move { join!(s1, s2, s3, s4) });

    tokio::time::sleep(Duration::from_millis(30)).await;

    Ok((
        ExampleClientBuilder::new("example1")
            .address(Address::from(u1))
            .build(),
        ExampleClientBuilder::new("example2")
            .address(Address::from(u2))
            .build(),
        ExampleClientBuilder::new("example3")
            .address(Address::from(u3))
            .build(),
        ExampleClientBuilder::new("example4")
            .address(Address::from(u4))
            .build(),
    ))
}

fn input() -> Input {
    Input {
        id: 1,
        desc: "one".into(),
    }
}

fn meta<T>(r: &Response<T>) -> String {
    format!("{:?}", r.metadata())
}

fn data<T>(r: &Response<T>) -> &T {
    r.get_ref()
}

async fn stream(r: Response<impl Stream<Item = Result<Output, Status>>>) -> Vec<Output> {
    r.into_inner().collect::<Result<Vec<_>, _>>().await.unwrap()
}

fn status(s: &volo_grpc::Status) -> (String, volo_grpc::Code) {
    (format!("{:?}", s.metadata()), s.code())
}
