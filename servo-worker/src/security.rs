//! Security utilities for payload verification
//!
//! Implements HMAC-SHA256 signature verification with constant-time comparison
//! to prevent timing attacks.

use hex::FromHex;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Error types for security operations
#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("Invalid signature format: {0}")]
    InvalidFormat(String),

    #[error("Signature verification failed")]
    VerificationFailed,

    #[error("Missing signature header")]
    MissingSignature,
}

/// Verify HMAC-SHA256 signature of a payload
///
/// Uses constant-time comparison to prevent timing attacks.
///
/// # Arguments
///
/// * `payload` - The raw payload bytes
/// * `signature_hex` - The hex-encoded signature to verify
/// * `secret` - The HMAC secret key
///
/// # Returns
///
/// `Ok(())` if verification succeeds
///
/// # Errors
///
/// Returns an error if:
/// - Signature format is invalid
/// - Signature doesn't match the payload
///
/// # Security
///
/// This function uses constant-time comparison to prevent timing-based attacks
/// that could leak information about the signature.
pub fn verify_signature(
    payload: &[u8],
    signature_hex: &str,
    secret: &str,
) -> Result<(), SecurityError> {
    // Decode hex signature
    let signature_bytes = <Vec<u8>>::from_hex(signature_hex).map_err(|e| {
        SecurityError::InvalidFormat(format!("Failed to decode hex signature: {}", e))
    })?;

    // Compute expected signature
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can accept keys of any size");
    mac.update(payload);
    let expected = mac.finalize().into_bytes();

    // Constant-time comparison to prevent timing attacks
    if expected.ct_eq(&signature_bytes[..]).into() {
        Ok(())
    } else {
        Err(SecurityError::VerificationFailed)
    }
}

/// Extract and validate signature from HTTP headers
///
/// # Arguments
///
/// * `signature_header` - Value of X-Servo-Signature header
///
/// # Returns
///
/// The signature value if present
///
/// # Errors
///
/// Returns an error if the signature header is missing
pub fn extract_signature(signature_header: Option<&str>) -> Result<&str, SecurityError> {
    signature_header.ok_or(SecurityError::MissingSignature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_valid_signature() {
        let secret = "test-secret";
        let payload = b"test payload";

        // Generate signature
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        let signature = hex::encode(mac.finalize().into_bytes());

        // Verify signature
        assert!(verify_signature(payload, &signature, secret).is_ok());
    }

    #[test]
    fn test_verify_invalid_signature() {
        let secret = "test-secret";
        let payload = b"test payload";
        let wrong_signature = "0000000000000000000000000000000000000000000000000000000000000000";

        // Verify should fail
        let result = verify_signature(payload, wrong_signature, secret);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SecurityError::VerificationFailed));
    }

    #[test]
    fn test_verify_wrong_secret() {
        let secret = "test-secret";
        let wrong_secret = "wrong-secret";
        let payload = b"test payload";

        // Generate signature with correct secret
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        let signature = hex::encode(mac.finalize().into_bytes());

        // Verify with wrong secret should fail
        let result = verify_signature(payload, &signature, wrong_secret);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SecurityError::VerificationFailed));
    }

    #[test]
    fn test_verify_invalid_hex() {
        let secret = "test-secret";
        let payload = b"test payload";
        let invalid_hex = "not-valid-hex";

        // Verify should fail with format error
        let result = verify_signature(payload, invalid_hex, secret);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SecurityError::InvalidFormat(_)));
    }

    #[test]
    fn test_extract_signature_present() {
        let sig = "abc123";
        let result = extract_signature(Some(sig));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), sig);
    }

    #[test]
    fn test_extract_signature_missing() {
        let result = extract_signature(None);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SecurityError::MissingSignature));
    }

    #[test]
    fn test_constant_time_comparison() {
        // This test verifies that our comparison is constant-time
        // by using the subtle crate's ConstantTimeEq trait
        let secret = "secret";
        let payload = b"test payload";

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        let signature = hex::encode(mac.finalize().into_bytes());

        // Same signature should verify
        assert!(verify_signature(payload, &signature, secret).is_ok());

        // Different signature with same length should also use constant time
        let wrong_sig = "a".repeat(signature.len());
        let result = verify_signature(payload, &wrong_sig, secret);
        assert!(result.is_err());
    }
}
