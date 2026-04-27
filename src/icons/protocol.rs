use std::borrow::Cow;

use wry::http::{Response, StatusCode, header::CONTENT_TYPE};

const ICON_PROTOCOL_SCHEME: &str = "runx";
const ICON_PROTOCOL_HOST: &str = "localhost";

pub(super) fn icon_protocol_url(icon_key: &str) -> String {
    format!("{ICON_PROTOCOL_SCHEME}://{ICON_PROTOCOL_HOST}/icon/{icon_key}.webp")
}

pub(super) fn icon_key_from_request_path(path: &str) -> Option<&str> {
    let key = path.strip_prefix("/icon/")?.strip_suffix(".webp")?;
    key.chars().all(|ch| ch.is_ascii_hexdigit()).then_some(key)
}

pub(super) fn response_with_status(
    status: StatusCode,
    content_type: &'static str,
    body: &'static [u8],
) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(Cow::Borrowed(body))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(body)))
}

#[cfg(test)]
mod tests {
    use super::icon_key_from_request_path;

    #[test]
    fn extracts_icon_key_from_protocol_path() {
        assert_eq!(
            icon_key_from_request_path("/icon/deadbeef00cafe42.webp"),
            Some("deadbeef00cafe42")
        );
    }

    #[test]
    fn rejects_non_icon_protocol_paths() {
        assert!(icon_key_from_request_path("/icons/deadbeef.webp").is_none());
        assert!(icon_key_from_request_path("/icon/not-hex.webp").is_none());
        assert!(icon_key_from_request_path("/icon/deadbeef.svg").is_none());
    }
}
