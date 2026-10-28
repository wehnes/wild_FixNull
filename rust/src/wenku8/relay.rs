//! Request compatibility with the officially released MewX Android v1.30 client.
//!
//! Its native version helper signs a UTC minute window. A plain app version is
//! rejected by the relay even when the remaining chapter parameters are valid.

use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

const CLIENT_VERSION: &str = "1.30";
const RELEASE_LABEL: &str = "digital-bento";

fn signed_app_version(timestamp_millis: i64) -> String {
    let minute = timestamp_millis.div_euclid(60_000);
    let message = format!("{CLIENT_VERSION}|{RELEASE_LABEL}|{minute}");
    let mut mac = Hmac::<Sha256>::new_from_slice(RELEASE_LABEL.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(message.as_bytes());
    let digest = hex::encode(mac.finalize().into_bytes());
    format!("{CLIENT_VERSION}-{RELEASE_LABEL}-{}", &digest[12..20])
}

pub(super) fn chapter_request_params(
    aid: &str,
    cid: &str,
    timestamp_millis: i64,
) -> [(&'static str, String); 3] {
    let request = format!("action=book&do=text&aid={aid}&cid={cid}&t=0");
    [
        (
            "request",
            base64::prelude::BASE64_STANDARD.encode(request.as_bytes()),
        ),
        ("appver", signed_app_version(timestamp_millis)),
        ("timetoken", timestamp_millis.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_released_client_reference_vectors() {
        assert_eq!(signed_app_version(0), "1.30-digital-bento-6d5bc8fd");
        assert_eq!(signed_app_version(60_000), "1.30-digital-bento-0e9fc421");
        assert_eq!(
            signed_app_version(1_793_162_218_000),
            "1.30-digital-bento-e835113f"
        );
    }

    #[test]
    fn uses_utc_minute_windows() {
        assert_eq!(signed_app_version(0), signed_app_version(59_999));
        assert_ne!(signed_app_version(59_999), signed_app_version(60_000));
    }

    #[test]
    fn chapter_form_keeps_ids_language_and_millisecond_timestamp() {
        let params = chapter_request_params("1587", "180393", 1_793_162_218_000);
        let decoded = base64::prelude::BASE64_STANDARD
            .decode(&params[0].1)
            .unwrap();
        assert_eq!(decoded, b"action=book&do=text&aid=1587&cid=180393&t=0");
        assert_eq!(params[1].1, "1.30-digital-bento-e835113f");
        assert_eq!(params[2].1, "1793162218000");
    }
}
