//! HMAC signing utilities for task payloads.

use crate::{Error, Result};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Sign a payload with HMAC-SHA256, returning a hex-encoded signature.
pub fn sign_payload(payload: &[u8], secret: &str) -> Result<String> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| Error::Configuration("Invalid HMAC secret".into()))?;
    mac.update(payload);
    let signature = mac.finalize().into_bytes();
    Ok(hex::encode(signature))
}

/// Verify an HMAC-SHA256 signature for the given payload.
pub fn verify_signature(payload: &[u8], signature: &str, secret: &str) -> Result<bool> {
    let expected = sign_payload(payload, secret)?;
    // Constant-time comparison
    Ok(subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), signature.as_bytes()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        let secret = "super-secret";
        let payload = br#"{"hello":"world"}"#;
        let sig = sign_payload(payload, secret).unwrap();
        assert!(verify_signature(payload, &sig, secret).unwrap());
    }

    #[test]
    fn verify_fails_with_wrong_secret() {
        let payload = br#"{"hello":"world"}"#;
        let sig = sign_payload(payload, "secret1").unwrap();
        assert!(!verify_signature(payload, &sig, "secret2").unwrap());
    }
}
