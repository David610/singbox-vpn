//! Certificate pinning shared by both transports. Relays are operator-run,
//! not public web servers, so we pin the exact certificate distributed in
//! the signed rendezvous bundle rather than trusting a public CA — this
//! avoids depending on the public WebPKI (and its own set of trust/
//! revocation issues) for a private relay fleet. The handshake signature
//! itself is still fully verified (not skipped) — only the "is this cert
//! in a CA chain" check is replaced with "is this the exact pinned cert".

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};
use sha2::{Digest, Sha256};

pub fn sha256_of_cert(der: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(der);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

#[derive(Debug)]
pub struct PinnedCertVerifier {
    expected_sha256: [u8; 32],
    supported_algs: WebPkiSupportedAlgorithms,
}

impl PinnedCertVerifier {
    pub fn new(expected_sha256: [u8; 32]) -> Self {
        let provider = rustls::crypto::ring::default_provider();
        Self {
            expected_sha256,
            supported_algs: provider.signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let digest = sha256_of_cert(end_entity.as_ref());
        if digest != self.expected_sha256 {
            return Err(TlsError::General("certificate pin mismatch".into()));
        }
        // Matching the exact pinned bytes proves this is the certificate the
        // operator distributed via the signed rendezvous bundle, but it does
        // not prove that certificate is still within its validity window —
        // that discarded `now: UnixTime` used to be ignored entirely, so a
        // pinned certificate never expired. Revocation still depends on
        // rotating the bundle (see the module docs), but expiry itself is
        // just a timestamp comparison and doesn't need that.
        let (not_before, not_after) = validity::parse(end_entity.as_ref())?;
        let now_secs = now.as_secs();
        if now_secs < not_before {
            return Err(TlsError::General(
                "pinned certificate is not yet valid (notBefore in the future)".into(),
            ));
        }
        if now_secs > not_after {
            return Err(TlsError::General(
                "pinned certificate has expired (past notAfter)".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.supported_algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.supported_algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported_algs.supported_schemes()
    }
}

/// Minimal ASN.1 DER walker that extracts only `tbsCertificate.validity`
/// (`notBefore`/`notAfter`) from a raw X.509 certificate, and nothing else.
///
/// This deliberately does NOT attempt full certificate parsing or chain
/// validation — this verifier already trusts the exact bytes matched by
/// SHA-256 above, so there is no chain to build and no other field this
/// verifier needs. Pulling in a general X.509 parsing dependency for one
/// timestamp pair was judged not worth the added dependency surface;
/// every byte access below is bounds-checked (`Option`/`Result`, never
/// indexing or `unwrap`), so a malformed or unexpected structure is a
/// parse error — fail closed — never a panic.
mod validity {
    use rustls::Error as TlsError;

    fn bad() -> TlsError {
        TlsError::General("malformed certificate DER (validity parse)".into())
    }

    /// Reads one DER TLV at `pos`. Returns (tag, content_start, content_len, next_pos).
    fn tlv(buf: &[u8], pos: usize) -> Result<(u8, usize, usize, usize), TlsError> {
        let tag = *buf.get(pos).ok_or_else(bad)?;
        let len_byte = *buf.get(pos + 1).ok_or_else(bad)?;
        let (len, header_len): (usize, usize) = if len_byte & 0x80 == 0 {
            (len_byte as usize, 2)
        } else {
            let n = (len_byte & 0x7f) as usize;
            // Reject indefinite-length (n == 0, not valid in DER anyway) and
            // anything that couldn't fit a certificate-sized length in usize
            // on any real platform.
            if n == 0 || n > 4 {
                return Err(bad());
            }
            let mut len = 0usize;
            for i in 0..n {
                len = (len << 8) | *buf.get(pos + 2 + i).ok_or_else(bad)? as usize;
            }
            (len, 2 + n)
        };
        let content_start = pos.checked_add(header_len).ok_or_else(bad)?;
        let content_end = content_start.checked_add(len).ok_or_else(bad)?;
        if content_end > buf.len() {
            return Err(bad());
        }
        Ok((tag, content_start, len, content_end))
    }

    // Howard Hinnant's days-from-civil algorithm (proleptic Gregorian
    // calendar, agrees with the Unix epoch at 1970-01-01) — avoids pulling
    // in a calendar/date dependency just to turn Y/M/D into a day count.
    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400; // [0, 399]
        let mp = (m + 9) % 12; // [0, 11]
        let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
        era * 146097 + doe - 719468
    }

    /// Parses an X.509 `Time` (UTCTime, tag 0x17, `YYMMDDHHMMSSZ`; or
    /// GeneralizedTime, tag 0x18, `YYYYMMDDHHMMSSZ`) into Unix seconds.
    /// Only the always-present, zero-fractional-second `Z`-suffixed forms
    /// required by RFC 5280 are accepted; anything else is a parse error.
    fn parse_time(tag: u8, bytes: &[u8]) -> Result<u64, TlsError> {
        let s = core::str::from_utf8(bytes).map_err(|_| bad())?;
        let (year, rest) = match tag {
            0x17 => {
                if s.len() != 13 || !s.ends_with('Z') {
                    return Err(bad());
                }
                let yy: i64 = s[0..2].parse().map_err(|_| bad())?;
                // RFC 5280 §4.1.2.5.1: 50-99 => 19xx, 00-49 => 20xx.
                let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
                (year, &s[2..12])
            }
            0x18 => {
                if s.len() != 15 || !s.ends_with('Z') {
                    return Err(bad());
                }
                let year: i64 = s[0..4].parse().map_err(|_| bad())?;
                (year, &s[4..14])
            }
            _ => return Err(bad()),
        };
        let month: i64 = rest[0..2].parse().map_err(|_| bad())?;
        let day: i64 = rest[2..4].parse().map_err(|_| bad())?;
        let hour: i64 = rest[4..6].parse().map_err(|_| bad())?;
        let min: i64 = rest[6..8].parse().map_err(|_| bad())?;
        let sec: i64 = rest[8..10].parse().map_err(|_| bad())?;
        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || min > 59
            || sec > 60
        {
            return Err(bad());
        }
        let days = days_from_civil(year, month, day);
        let secs = days * 86400 + hour * 3600 + min * 60 + sec;
        u64::try_from(secs).map_err(|_| bad())
    }

    /// Returns `(not_before, not_after)` as Unix seconds.
    pub fn parse(cert_der: &[u8]) -> Result<(u64, u64), TlsError> {
        // Certificate ::= SEQUENCE { tbsCertificate TBSCertificate, ... }
        let (tag, cert_start, _, _) = tlv(cert_der, 0)?;
        if tag != 0x30 {
            return Err(bad());
        }
        // TBSCertificate ::= SEQUENCE { version?, serialNumber, signature,
        //                               issuer, validity, subject, ... }
        let (tag, mut pos, _, _) = tlv(cert_der, cert_start)?;
        if tag != 0x30 {
            return Err(bad());
        }
        // Optional [0] EXPLICIT Version — present in every v2/v3 cert, but
        // not in a bare v1 cert, so detect rather than assume.
        let (tag, _, _, next) = tlv(cert_der, pos)?;
        if tag == 0xA0 {
            pos = next;
        }
        // serialNumber INTEGER
        let (tag, _, _, next) = tlv(cert_der, pos)?;
        if tag != 0x02 {
            return Err(bad());
        }
        // signature AlgorithmIdentifier SEQUENCE
        let (tag, _, _, next) = tlv(cert_der, next)?;
        if tag != 0x30 {
            return Err(bad());
        }
        // issuer Name SEQUENCE
        let (tag, _, _, next) = tlv(cert_der, next)?;
        if tag != 0x30 {
            return Err(bad());
        }
        // validity Validity ::= SEQUENCE { notBefore Time, notAfter Time }
        let (tag, validity_start, _, _) = tlv(cert_der, next)?;
        if tag != 0x30 {
            return Err(bad());
        }
        let (nb_tag, nb_start, nb_len, nb_end) = tlv(cert_der, validity_start)?;
        let not_before = parse_time(nb_tag, &cert_der[nb_start..nb_start + nb_len])?;
        let (na_tag, na_start, na_len, _) = tlv(cert_der, nb_end)?;
        let not_after = parse_time(na_tag, &cert_der[na_start..na_start + na_len])?;
        Ok((not_before, not_after))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::cert::generate_self_signed;
        use rcgen::{CertificateParams, KeyPair};
        use time::macros::datetime;

        #[test]
        fn parses_rcgen_default_wide_validity_window() {
            let (cert, _key) = generate_self_signed("example.invalid");
            let (not_before, not_after) = parse(cert.der()).expect("parse validity");
            // rcgen's CertificateParams::default() uses 1975-01-01 /
            // 4096-01-01 — both well outside UTCTime's 1950-2049 window
            // (exercising the UTCTime and GeneralizedTime branches
            // respectively), so this doubles as a round-trip check that
            // both Time encodings are parsed correctly.
            assert_eq!(
                not_before,
                datetime!(1975-01-01 00:00:00 UTC).unix_timestamp() as u64
            );
            assert_eq!(
                not_after,
                datetime!(4096-01-01 00:00:00 UTC).unix_timestamp() as u64
            );
        }

        #[test]
        fn narrow_validity_window_round_trips() {
            let key_pair = KeyPair::generate().expect("keygen");
            let mut params =
                CertificateParams::new(vec!["example.invalid".to_string()]).expect("params");
            params.not_before = datetime!(2024-06-15 12:30:00 UTC);
            params.not_after = datetime!(2024-07-15 12:30:00 UTC);
            let cert = params.self_signed(&key_pair).expect("self-sign");
            let (not_before, not_after) = parse(cert.der()).expect("parse validity");
            assert_eq!(
                not_before,
                datetime!(2024-06-15 12:30:00 UTC).unix_timestamp() as u64
            );
            assert_eq!(
                not_after,
                datetime!(2024-07-15 12:30:00 UTC).unix_timestamp() as u64
            );
        }

        #[test]
        fn rejects_truncated_der() {
            let (cert, _key) = generate_self_signed("example.invalid");
            let der = cert.der();
            for cut in [0usize, 1, 4, der.len() / 2] {
                assert!(
                    parse(&der[..cut]).is_err(),
                    "truncation at {cut} must not panic or succeed"
                );
            }
        }
    }
}

/// Generates a fresh self-signed keypair+certificate for a relay to serve.
/// Dev/local-slice tooling only — a real deployment provisions relay
/// certificates out of band and distributes the pin via the signed
/// rendezvous bundle (see RENDEZVOUS_DESIGN.md); this function is used by
/// `deploy/local/run-dev-slice.sh`'s in-process components and test fixtures.
pub fn generate_self_signed(subject_alt_name: &str) -> (rcgen::Certificate, rcgen::KeyPair) {
    let key_pair = rcgen::KeyPair::generate().expect("keygen");
    let params = rcgen::CertificateParams::new(vec![subject_alt_name.to_string()]).expect("params");
    let cert = params.self_signed(&key_pair).expect("self-sign");
    (cert, key_pair)
}
