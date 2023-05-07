use std::{
    collections::{BTreeSet, HashSet},
    convert::TryFrom,
    fmt::Debug,
    sync::Arc,
    time::Duration,
};

pub(crate) use http::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS as ALLOW_CREDENTIALS,
    ACCESS_CONTROL_ALLOW_HEADERS as ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS as ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN as ALLOW_ORIGIN, ACCESS_CONTROL_EXPOSE_HEADERS as EXPOSE_HEADERS,
    ACCESS_CONTROL_MAX_AGE as MAX_AGE, ACCESS_CONTROL_REQUEST_HEADERS as REQUEST_HEADERS,
    ACCESS_CONTROL_REQUEST_METHOD as REQUEST_METHOD,
};
use http::{
    header::{self, HeaderName},
    HeaderMap, HeaderValue, Method,
};
use tracing::debug;

const DEFAULT_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

const DEFAULT_EXPOSED_HEADERS: [&str; 2] = ["grpc-status", "grpc-message"];
const DEFAULT_ALLOWED_METHODS: &[Method; 2] = &[Method::POST, Method::OPTIONS];

#[derive(Debug, PartialEq)]
pub(crate) enum Error {
    OriginNotAllowed,
    MethodNotAllowed,
}

#[derive(Debug, Clone)]
pub(crate) enum AllowedOrigins {
    Any,
    Only(BTreeSet<HeaderValue>),
}

impl AllowedOrigins {
    pub(crate) fn is_allowed(&self, origin: &HeaderValue) -> bool {
        match self {
            AllowedOrigins::Any => true,
            AllowedOrigins::Only(origins) => origins.contains(origin),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    allowed_origins: AllowedOrigins,
    exposed_headers: HashSet<HeaderName>,
    max_age: Option<Duration>,
    allow_credentials: bool,
}

impl Config {
    pub fn new() -> Self {
        Config {
            allowed_origins: AllowedOrigins::Any,
            exposed_headers: DEFAULT_EXPOSED_HEADERS
                .iter()
                .copied()
                .map(HeaderName::from_static)
                .collect(),
            max_age: Some(DEFAULT_MAX_AGE),
            allow_credentials: true,
        }
    }

    #[allow(clippy::mutable_key_type)]
    #[must_use]
    pub fn allow_origins<I>(self, origins: I) -> Self
    where
        I: IntoIterator,
        HeaderValue: TryFrom<I::Item>,
        <HeaderValue as TryFrom<I::Item>>::Error: Debug,
    {
        let origins = origins
            .into_iter()
            .map(|v| TryFrom::try_from(v).expect("invalid origin"))
            .collect();

        Self {
            allowed_origins: AllowedOrigins::Only(origins),
            ..self
        }
    }

    #[must_use]
    pub fn expose_headers<I>(mut self, headers: I) -> Self
    where
        I: IntoIterator,
        HeaderName: TryFrom<I::Item>,
        <HeaderName as TryFrom<I::Item>>::Error: Debug,
    {
        let iter = headers
            .into_iter()
            .map(|header| TryFrom::try_from(header).expect("invalid header"));

        self.exposed_headers.extend(iter);
        self
    }

    #[must_use]
    pub fn max_age<T: Into<Option<Duration>>>(self, max_age: T) -> Self {
        Self {
            max_age: max_age.into(),
            ..self
        }
    }

    #[must_use]
    pub fn allow_credentials(self, allow_credentials: bool) -> Self {
        Self {
            allow_credentials,
            ..self
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::new()
    }
}

#[derive(Clone, Debug)]
pub struct Cors {
    inner: Arc<Config>,
}

impl Cors {
    pub fn new(config: Config) -> Self {
        Self {
            inner: Arc::new(config),
        }
    }

    pub(crate) fn simple(&self, headers: &HeaderMap) -> Result<HeaderMap, Error> {
        match headers.get(header::ORIGIN) {
            Some(origin) if self.inner.allowed_origins.is_allowed(origin) => {
                Ok(self.common_headers(origin.clone()))
            }
            Some(_) => Err(Error::OriginNotAllowed),
            None => Ok(HeaderMap::new()),
        }
    }

    pub(crate) fn preflight(
        &self,
        req_headers: &HeaderMap,
        origin: &HeaderValue,
        request_headers_header: &HeaderValue,
    ) -> Result<HeaderMap, Error> {
        if !self.inner.allowed_origins.is_allowed(origin) {
            return Err(Error::OriginNotAllowed);
        }

        if !is_method_allowed(req_headers.get(REQUEST_METHOD)) {
            return Err(Error::MethodNotAllowed);
        }

        let mut headers = self.common_headers(origin.clone());
        headers.insert(ALLOW_METHODS, HeaderValue::from_static("POST,OPTIONS"));
        headers.insert(ALLOW_HEADERS, request_headers_header.clone());

        if let Some(max_age) = self.inner.max_age {
            headers.insert(MAX_AGE, HeaderValue::from(max_age.as_secs()));
        }

        Ok(headers)
    }

    fn common_headers(&self, origin: HeaderValue) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(ALLOW_ORIGIN, origin);
        headers.insert(
            EXPOSE_HEADERS,
            join_header_value(&self.inner.exposed_headers).unwrap(),
        );

        if self.inner.allow_credentials {
            headers.insert(ALLOW_CREDENTIALS, HeaderValue::from_static("true"));
        }

        headers
    }
}

fn is_method_allowed(header: Option<&HeaderValue>) -> bool {
    if let Some(value) = header {
        if let Ok(method) = Method::from_bytes(value.as_bytes()) {
            DEFAULT_ALLOWED_METHODS.contains(&method)
        } else {
            debug!("access-control-request-method {:?} is not valid", value);
            false
        }
    } else {
        debug!("access-control-request-method is missing");
        false
    }
}

fn join_header_value<I>(values: I) -> Result<HeaderValue, header::InvalidHeaderValue>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut values = values.into_iter();
    let mut value = Vec::new();

    if let Some(v) = values.next() {
        value.extend(v.as_ref().as_bytes());
    }
    for v in values {
        value.push(b',');
        value.extend(v.as_ref().as_bytes());
    }
    HeaderValue::from_bytes(&value)
}
