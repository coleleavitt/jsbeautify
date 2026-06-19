use bytes::Bytes;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use serde::Serialize;

pub(super) fn cors(mut resp: Response<Full<Bytes>>) -> Response<Full<Bytes>> {
    let h = resp.headers_mut();
    h.insert("access-control-allow-origin", "*".parse().expect("static"));
    h.insert(
        "access-control-allow-methods",
        "GET, POST, OPTIONS".parse().expect("static"),
    );
    h.insert("access-control-allow-headers", "content-type".parse().expect("static"));
    resp
}

pub(super) fn json_ok<T: Serialize>(value: &T) -> Response<Full<Bytes>> {
    match serde_json::to_vec(value) {
        Ok(body) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "build response")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &format!("serialize: {e}")),
    }
}

pub(super) fn no_content() -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Full::new(Bytes::new()))
        .expect("static response always builds")
}

pub(super) fn error(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    let body = serde_json::json!({ "ok": false, "error": msg }).to_string();
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())))
}
