use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};

pub(super) fn bad_request(msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .body(Full::new(Bytes::from(format!("400 Bad Request: {msg}"))))
        .expect("static response always builds")
}

pub(super) fn bad_gateway(msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .body(Full::new(Bytes::from(format!("502 Bad Gateway: {msg}"))))
        .expect("static response always builds")
}
