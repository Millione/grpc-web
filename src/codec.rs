use std::{
    error::Error,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures_core::{ready, Stream};
use http::{header, HeaderMap, HeaderValue};
use http_body::{Body, SizeHint};
use pin_project::pin_project;
use volo_grpc::Status;

use crate::{GRPC_WEB_PROTO, GRPC_WEB_TEXT, GRPC_WEB_TEXT_PROTO};

const BUFFER_SIZE: usize = 8 * 1024;

const FRAME_HEADER_SIZE: usize = 5;

const GRPC_WEB_TRAILERS_BIT: u8 = 0b10000000;

#[derive(Copy, Clone, PartialEq, Debug)]
enum Direction {
    Request,
    Response,
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub(crate) enum Encoding {
    Base64,
    None,
}

#[pin_project]
pub(crate) struct WebCall<B> {
    #[pin]
    inner: B,
    buf: BytesMut,
    direction: Direction,
    encoding: Encoding,
    poll_trailers: bool,
}

impl<B> WebCall<B> {
    pub(crate) fn request(inner: B, encoding: Encoding) -> Self {
        Self::new(inner, Direction::Request, encoding)
    }

    pub(crate) fn response(inner: B, encoding: Encoding) -> Self {
        Self::new(inner, Direction::Response, encoding)
    }

    fn new(inner: B, direction: Direction, encoding: Encoding) -> Self {
        WebCall {
            inner,
            buf: BytesMut::with_capacity(match (direction, encoding) {
                (Direction::Response, Encoding::Base64) => BUFFER_SIZE,
                _ => 0,
            }),
            direction,
            encoding,
            poll_trailers: true,
        }
    }

    #[inline]
    fn max_decodable(&self) -> usize {
        (self.buf.len() / 4) * 4
    }

    fn decode_chunk(mut self: Pin<&mut Self>) -> Result<Option<Bytes>, Status> {
        if self.buf.is_empty() || self.buf.len() < 4 {
            return Ok(None);
        }

        let index = self.max_decodable();

        base64::decode(self.as_mut().project().buf.split_to(index))
            .map(|decoded| Some(Bytes::from(decoded)))
            .map_err(internal_error)
    }
}

impl<B> WebCall<B>
where
    B: Body<Data = Bytes>,
    B::Error: Error,
{
    fn poll_decode(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<B::Data, Status>>> {
        match self.encoding {
            Encoding::Base64 => loop {
                if let Some(bytes) = self.as_mut().decode_chunk()? {
                    return Poll::Ready(Some(Ok(bytes)));
                }

                let mut this = self.as_mut().project();

                match ready!(this.inner.as_mut().poll_data(cx)) {
                    Some(Ok(data)) => this.buf.put(data),
                    Some(Err(e)) => return Poll::Ready(Some(Err(internal_error(e)))),
                    None => {
                        return if this.buf.has_remaining() {
                            Poll::Ready(Some(Err(internal_error("malformed base64 request"))))
                        } else {
                            Poll::Ready(None)
                        }
                    }
                }
            },

            Encoding::None => match ready!(self.project().inner.poll_data(cx)) {
                Some(res) => Poll::Ready(Some(res.map_err(internal_error))),
                None => Poll::Ready(None),
            },
        }
    }

    fn poll_encode(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<B::Data, Status>>> {
        let mut this = self.as_mut().project();

        if let Some(mut res) = ready!(this.inner.as_mut().poll_data(cx)) {
            if *this.encoding == Encoding::Base64 {
                res = res.map(|b| base64::encode(b).into())
            }

            return Poll::Ready(Some(res.map_err(internal_error)));
        }

        if *this.poll_trailers {
            return match ready!(this.inner.poll_trailers(cx)) {
                Ok(Some(map)) => {
                    let mut frame = make_trailers_frame(map);

                    if *this.encoding == Encoding::Base64 {
                        frame = base64::encode(frame).into_bytes();
                    }

                    *this.poll_trailers = false;
                    Poll::Ready(Some(Ok(frame.into())))
                }
                Ok(None) => Poll::Ready(None),
                Err(e) => Poll::Ready(Some(Err(internal_error(e)))),
            };
        }

        Poll::Ready(None)
    }
}

impl<B> Body for WebCall<B>
where
    B: Body<Data = Bytes>,
    B::Error: Error,
{
    type Data = Bytes;
    type Error = Status;

    fn poll_data(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        match self.direction {
            Direction::Request => self.poll_decode(cx),
            Direction::Response => self.poll_encode(cx),
        }
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Result<Option<HeaderMap<HeaderValue>>, Self::Error>> {
        Poll::Ready(Ok(None))
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl<B> Stream for WebCall<B>
where
    B: Body<Data = Bytes>,
    B::Error: Error,
{
    type Item = Result<Bytes, Status>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Body::poll_data(self, cx)
    }
}

impl Encoding {
    pub(crate) fn from_content_type(headers: &HeaderMap) -> Encoding {
        Self::from_header(headers.get(header::CONTENT_TYPE))
    }

    pub(crate) fn from_accept(headers: &HeaderMap) -> Encoding {
        Self::from_header(headers.get(header::ACCEPT))
    }

    pub(crate) fn to_content_type(self) -> &'static str {
        match self {
            Encoding::Base64 => GRPC_WEB_TEXT_PROTO,
            Encoding::None => GRPC_WEB_PROTO,
        }
    }

    fn from_header(value: Option<&HeaderValue>) -> Encoding {
        match value.and_then(|val| val.to_str().ok()) {
            Some(GRPC_WEB_TEXT_PROTO) | Some(GRPC_WEB_TEXT) => Encoding::Base64,
            _ => Encoding::None,
        }
    }
}

#[inline]
fn internal_error(e: impl std::fmt::Display) -> Status {
    Status::internal(format!("grpc-web: {e}"))
}

fn make_trailers_frame(trailers: HeaderMap) -> Vec<u8> {
    let trailers = trailers.iter().fold(Vec::new(), |mut acc, (key, value)| {
        acc.put_slice(key.as_ref());
        acc.push(b':');
        acc.put_slice(value.as_bytes());
        acc.put_slice(b"\r\n");
        acc
    });
    let len = trailers.len();
    assert!(len <= u32::MAX as usize);

    let mut frame = Vec::with_capacity(len + FRAME_HEADER_SIZE);
    frame.push(GRPC_WEB_TRAILERS_BIT);
    frame.put_u32(len as u32);
    frame.extend(trailers);

    frame
}
