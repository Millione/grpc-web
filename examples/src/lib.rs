#![feature(impl_trait_in_assoc_type)]

pub mod volo_gen {
    volo::include_service!("proto_gen.rs");
}

use tokio_stream::StreamExt;
use volo_gen::proto_gen::example::{Input, Output};
use volo_grpc::{BoxStream, RecvStream, Request, Response, Status};

use crate::volo_gen::proto_gen::example::Example;

pub struct S;

#[async_trait::async_trait]
impl Example for S {
    async fn unary_call(&self, req: Request<Input>) -> Result<Response<Output>, Status> {
        let req = req.into_inner();

        if &req.desc == "boom" {
            Err(Status::invalid_argument("invalid boom"))
        } else {
            Ok(Response::new(Output {
                id: req.id,
                desc: req.desc,
            }))
        }
    }

    async fn client_stream(
        &self,
        req: Request<RecvStream<Input>>,
    ) -> Result<Response<Output>, Status> {
        let out = Output {
            id: 0,
            desc: "".into(),
        };

        Ok(Response::new(
            req.into_inner()
                .fold(out, |mut acc, input| {
                    let input = input.unwrap();
                    acc.id += input.id;
                    acc.desc = format!("{}{}", acc.desc, input.desc).into();
                    acc
                })
                .await,
        ))
    }

    async fn server_stream(
        &self,
        req: Request<Input>,
    ) -> Result<Response<BoxStream<'static, Result<Output, Status>>>, Status> {
        let req = req.into_inner();

        Ok(Response::new(Box::pin(tokio_stream::iter(vec![1, 2]).map(
            move |n| {
                Ok(Output {
                    id: req.id,
                    desc: format!("{}-{}", n, req.desc).into(),
                })
            },
        ))))
    }
}
