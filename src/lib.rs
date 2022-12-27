#![feature(type_alias_impl_trait)]

mod codec;
mod config;

use std::future::Future;

use codec::{Encoding, WebCall};
pub use config::Config;
use config::Cors;
use http::{
    header::{self, CONTENT_TYPE, ORIGIN},
    HeaderMap, Response, StatusCode, Version,
};
use hyper::{http::HeaderValue, Method};
use tracing::{debug, trace};
use volo::Service;
use volo_grpc::{body::Body, context::ServerContext, server::NamedService, Status};

use crate::config::REQUEST_HEADERS;

pub(crate) const GRPC_WEB: &str = "application/grpc-web";
pub(crate) const GRPC_WEB_PROTO: &str = "application/grpc-web+proto";
pub(crate) const GRPC_WEB_TEXT: &str = "application/grpc-web-text";
pub(crate) const GRPC_WEB_TEXT_PROTO: &str = "application/grpc-web-text+proto";

#[derive(Clone, Debug)]
pub struct WebService<S> {
    inner: S,
    cors: Cors,
}

impl<S> WebService<S> {
    pub fn new(inner: S, cors: Cors) -> Self {
        Self { inner, cors }
    }
}

impl<S> WebService<S>
where
    S: Service<ServerContext, http::Request<hyper::Body>, Response = http::Response<Body>>,
{
    fn no_content(
        &self,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<S::Response, S::Error>> {
        let mut res = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::new(Box::pin(futures::stream::empty())))
            .unwrap();

        res.headers_mut().extend(headers);

        async { Ok(res) }
    }

    fn response(&self, status: StatusCode) -> impl Future<Output = Result<S::Response, S::Error>> {
        let res = Response::builder()
            .status(status)
            .body(Body::new(Box::pin(futures::stream::empty())))
            .unwrap();
        async { Ok(res) }
    }
}

impl<S> Service<ServerContext, http::Request<hyper::Body>> for WebService<S>
where
    S: Service<ServerContext, http::Request<hyper::Body>, Response = http::Response<Body>>
        + Send
        + Sync
        + 'static,
    S::Error: Into<Status>,
{
    type Response = S::Response;

    type Error = S::Error;

    type Future<'cx> = impl Future<Output = Result<Self::Response, Self::Error>> + 'cx;

    fn call<'cx, 's>(
        &'s self,
        cx: &'cx mut ServerContext,
        req: http::Request<hyper::Body>,
    ) -> Self::Future<'cx>
    where
        's: 'cx,
    {
        async move {
            match RequestKind::new(req.headers(), req.method(), req.version()) {
                RequestKind::InFlight {
                    method: &Method::POST,
                    encoding,
                    accept,
                } => match self.cors.simple(req.headers()) {
                    Ok(headers) => {
                        trace!(kind = "inflight", path = ?req.uri().path(), ?encoding, ?accept);

                        let fut = self.inner.call(cx, coerce_request(req, encoding));

                        let mut res = coerce_response(fut.await?, accept);
                        res.headers_mut().extend(headers);
                        Ok(res)
                    }
                    Err(e) => {
                        debug!(kind = "inflight", error=?e, ?req);
                        self.response(StatusCode::FORBIDDEN).await
                    }
                },

                RequestKind::InFlight { .. } => {
                    debug!(kind = "inflight", error="method not allowed", method = ?req.method());
                    self.response(StatusCode::METHOD_NOT_ALLOWED).await
                }

                RequestKind::PreFlight {
                    origin,
                    request_headers,
                } => match self.cors.preflight(req.headers(), origin, request_headers) {
                    Ok(headers) => {
                        trace!(kind = "preflight", path = ?req.uri().path(), ?origin);
                        self.no_content(headers).await
                    }
                    Err(e) => {
                        debug!(kind = "preflight", error = ?e, ?req);
                        self.response(StatusCode::FORBIDDEN).await
                    }
                },

                RequestKind::Other(Version::HTTP_2) => {
                    debug!(kind = "other h2", content_type = ?req.headers().get(header::CONTENT_TYPE));
                    self.inner.call(cx, req).await
                }

                RequestKind::Other(_) => {
                    debug!(kind = "other h1", content_type = ?req.headers().get(header::CONTENT_TYPE));
                    self.response(StatusCode::BAD_REQUEST).await
                }
            }
        }
    }
}

fn coerce_request(
    mut req: http::Request<hyper::Body>,
    encoding: Encoding,
) -> http::Request<hyper::Body> {
    req.headers_mut().remove(header::CONTENT_LENGTH);

    req.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/grpc"),
    );

    req.headers_mut()
        .insert(header::TE, HeaderValue::from_static("trailers"));

    req.headers_mut().insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity,deflate,gzip"),
    );

    req.map(|b| WebCall::request(b, encoding))
        .map(hyper::Body::wrap_stream)
}

fn coerce_response(res: http::Response<Body>, encoding: Encoding) -> http::Response<Body> {
    let mut res = res
        .map(|b| WebCall::response(b, encoding))
        .map(|b| Body::new(Box::pin(b)));

    res.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(encoding.to_content_type()),
    );

    res
}

impl<S: NamedService> NamedService for WebService<S> {
    const NAME: &'static str = S::NAME;
}

#[derive(Debug, PartialEq)]
enum RequestKind<'a> {
    InFlight {
        method: &'a Method,
        encoding: Encoding,
        accept: Encoding,
    },
    PreFlight {
        origin: &'a HeaderValue,
        request_headers: &'a HeaderValue,
    },
    Other(http::Version),
}

impl<'a> RequestKind<'a> {
    fn new(headers: &'a HeaderMap, method: &'a Method, version: Version) -> Self {
        if matches!(
            headers.get(CONTENT_TYPE).and_then(|val| val.to_str().ok()),
            Some(GRPC_WEB) | Some(GRPC_WEB_PROTO) | Some(GRPC_WEB_TEXT) | Some(GRPC_WEB_TEXT_PROTO)
        ) {
            return RequestKind::InFlight {
                method,
                encoding: Encoding::from_content_type(headers),
                accept: Encoding::from_accept(headers),
            };
        }

        if let (&Method::OPTIONS, Some(origin), Some(value)) =
            (method, headers.get(ORIGIN), headers.get(REQUEST_HEADERS))
        {
            match value.to_str() {
                Ok(h) if h.contains("x-grpc-web") => {
                    return RequestKind::PreFlight {
                        origin,
                        request_headers: value,
                    };
                }
                _ => {}
            }
        }

        RequestKind::Other(version)
    }
}
