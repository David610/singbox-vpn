//! The `profile-title` header value sent to subscription importers.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use compat_config::render::{
    profile_title_header_value, validate_profile_title, DEFAULT_PROFILE_TITLE,
};

#[test]
fn the_default_title_is_the_product_name_and_is_sent_verbatim() {
    assert_eq!(DEFAULT_PROFILE_TITLE, "Arcana VPN");
    assert!(validate_profile_title(DEFAULT_PROFILE_TITLE).is_ok());
    assert_eq!(
        profile_title_header_value(DEFAULT_PROFILE_TITLE),
        "Arcana VPN"
    );
}

#[test]
fn non_ascii_titles_are_base64_encoded_in_the_form_importers_decode() {
    let value = profile_title_header_value("Тамара VPN");
    let encoded = value.strip_prefix("base64:").expect("base64: prefix");
    assert_eq!(
        String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap(),
        "Тамара VPN"
    );
}

/// A literal title that itself starts with `base64:` would be decoded by the
/// client into garbage, so it is encoded rather than sent as-is.
#[test]
fn a_title_that_looks_like_the_base64_marker_is_still_encoded() {
    let value = profile_title_header_value("base64:oops");
    let encoded = value.strip_prefix("base64:").unwrap();
    assert_eq!(STANDARD.decode(encoded).unwrap(), b"base64:oops");
}

#[test]
fn every_header_value_is_a_valid_http_header() {
    for title in ["Arcana VPN", "Тамара VPN", "a", &"x".repeat(64), "base64:x"] {
        let value = profile_title_header_value(title);
        assert!(
            value.bytes().all(|b| (0x20..0x7f).contains(&b)),
            "header value for {title:?} must be printable ASCII, got {value:?}"
        );
    }
}
