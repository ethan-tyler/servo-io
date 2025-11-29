//! PII (Personally Identifiable Information) filtering for check failure samples.
//!
//! This module provides utilities to detect and redact sensitive data before
//! it gets stored in check results or logged. This is critical for compliance
//! with privacy regulations (GDPR, CCPA, HIPAA).
//!
//! # Supported PII Types
//!
//! - Email addresses
//! - Social Security Numbers (SSN)
//! - Credit card numbers (major formats)
//! - Phone numbers (US formats)
//! - IP addresses
//!
//! # Usage
//!
//! ```rust
//! use servo_worker::pii::{PiiFilter, PiiFilterConfig};
//!
//! let filter = PiiFilter::new(PiiFilterConfig::default());
//! let redacted = filter.redact("Contact: john@example.com, SSN: 123-45-6789");
//! assert_eq!(redacted, "Contact: [EMAIL], SSN: [SSN]");
//! ```

use regex::Regex;
use std::sync::LazyLock;

/// Configuration for PII filtering behavior
#[derive(Debug, Clone)]
pub struct PiiFilterConfig {
    /// Redact email addresses
    pub redact_emails: bool,
    /// Redact Social Security Numbers
    pub redact_ssn: bool,
    /// Redact credit card numbers
    pub redact_credit_cards: bool,
    /// Redact phone numbers
    pub redact_phone_numbers: bool,
    /// Redact IP addresses
    pub redact_ip_addresses: bool,
    /// Maximum sample length after redaction (truncates if exceeded)
    pub max_sample_length: usize,
}

impl Default for PiiFilterConfig {
    fn default() -> Self {
        Self {
            redact_emails: true,
            redact_ssn: true,
            redact_credit_cards: true,
            redact_phone_numbers: true,
            redact_ip_addresses: true,
            max_sample_length: 1000,
        }
    }
}

/// Compiled regex patterns for PII detection
struct PiiPatterns {
    email: Regex,
    ssn: Regex,
    credit_card: Regex,
    phone: Regex,
    ipv4: Regex,
}

static PII_PATTERNS: LazyLock<PiiPatterns> = LazyLock::new(|| PiiPatterns {
    // Email: standard format
    email: Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}")
        .expect("Invalid email regex"),

    // SSN: XXX-XX-XXXX or XXXXXXXXX
    ssn: Regex::new(r"\b\d{3}-\d{2}-\d{4}\b|\b\d{9}\b").expect("Invalid SSN regex"),

    // Credit cards: major formats (Visa, MC, Amex, Discover)
    // 13-19 digits, optionally separated by spaces or dashes
    credit_card: Regex::new(r"\b(?:\d{4}[-\s]?){3}\d{1,7}\b|\b\d{13,19}\b")
        .expect("Invalid credit card regex"),

    // Phone: US formats (XXX) XXX-XXXX, XXX-XXX-XXXX, XXX.XXX.XXXX, XXXXXXXXXX
    phone: Regex::new(r"(?:\(\d{3}\)\s*\d{3}[-.]?\d{4}|\b\d{3}[-.]?\d{3}[-.]?\d{4}\b)")
        .expect("Invalid phone regex"),

    // IPv4 addresses
    ipv4: Regex::new(r"\b(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b")
        .expect("Invalid IPv4 regex"),
});

/// PII filter that redacts sensitive data from strings
#[derive(Debug, Clone)]
pub struct PiiFilter {
    config: PiiFilterConfig,
}

impl PiiFilter {
    /// Create a new PII filter with the given configuration
    pub fn new(config: PiiFilterConfig) -> Self {
        Self { config }
    }

    /// Create a PII filter with default settings (redact all PII types)
    pub fn default_filter() -> Self {
        Self::new(PiiFilterConfig::default())
    }

    /// Redact PII from a string based on configuration
    pub fn redact(&self, input: &str) -> String {
        let mut result = input.to_string();

        if self.config.redact_emails {
            result = PII_PATTERNS.email.replace_all(&result, "[EMAIL]").to_string();
        }

        if self.config.redact_ssn {
            result = PII_PATTERNS.ssn.replace_all(&result, "[SSN]").to_string();
        }

        if self.config.redact_credit_cards {
            result = PII_PATTERNS
                .credit_card
                .replace_all(&result, "[CARD]")
                .to_string();
        }

        if self.config.redact_phone_numbers {
            result = PII_PATTERNS.phone.replace_all(&result, "[PHONE]").to_string();
        }

        if self.config.redact_ip_addresses {
            result = PII_PATTERNS.ipv4.replace_all(&result, "[IP]").to_string();
        }

        // Truncate if exceeds max length
        if result.len() > self.config.max_sample_length {
            result.truncate(self.config.max_sample_length);
            result.push_str("...[TRUNCATED]");
        }

        result
    }

    /// Redact PII from a JSON value (recursively processes strings)
    pub fn redact_json(&self, value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(s) => serde_json::Value::String(self.redact(s)),
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(|v| self.redact_json(v)).collect())
            }
            serde_json::Value::Object(obj) => {
                let mut new_obj = serde_json::Map::new();
                for (k, v) in obj {
                    new_obj.insert(k.clone(), self.redact_json(v));
                }
                serde_json::Value::Object(new_obj)
            }
            // Numbers, bools, nulls pass through unchanged
            other => other.clone(),
        }
    }

    /// Redact PII from optional failure samples (convenience method)
    pub fn redact_samples(&self, samples: Option<serde_json::Value>) -> Option<serde_json::Value> {
        samples.map(|v| self.redact_json(&v))
    }
}

/// Global default PII filter instance
static DEFAULT_FILTER: LazyLock<PiiFilter> = LazyLock::new(PiiFilter::default_filter);

/// Redact PII using the default filter configuration
pub fn redact_pii(input: &str) -> String {
    DEFAULT_FILTER.redact(input)
}

/// Redact PII from JSON using the default filter configuration
pub fn redact_pii_json(value: &serde_json::Value) -> serde_json::Value {
    DEFAULT_FILTER.redact_json(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_redact_email() {
        let filter = PiiFilter::default_filter();
        assert_eq!(
            filter.redact("Contact: john.doe@example.com"),
            "Contact: [EMAIL]"
        );
        assert_eq!(
            filter.redact("Emails: a@b.co, test@domain.org"),
            "Emails: [EMAIL], [EMAIL]"
        );
    }

    #[test]
    fn test_redact_ssn() {
        let filter = PiiFilter::default_filter();
        assert_eq!(filter.redact("SSN: 123-45-6789"), "SSN: [SSN]");
        assert_eq!(filter.redact("SSN: 123456789"), "SSN: [SSN]");
    }

    #[test]
    fn test_redact_credit_card() {
        let filter = PiiFilter::default_filter();
        assert_eq!(
            filter.redact("Card: 4111-1111-1111-1111"),
            "Card: [CARD]"
        );
        assert_eq!(
            filter.redact("Card: 4111 1111 1111 1111"),
            "Card: [CARD]"
        );
        assert_eq!(
            filter.redact("Card: 4111111111111111"),
            "Card: [CARD]"
        );
    }

    #[test]
    fn test_redact_phone() {
        let filter = PiiFilter::default_filter();
        assert_eq!(filter.redact("Call: (555) 123-4567"), "Call: [PHONE]");
        assert_eq!(filter.redact("Call: 555-123-4567"), "Call: [PHONE]");
        assert_eq!(filter.redact("Call: 555.123.4567"), "Call: [PHONE]");
    }

    #[test]
    fn test_redact_ip_address() {
        let filter = PiiFilter::default_filter();
        assert_eq!(filter.redact("IP: 192.168.1.1"), "IP: [IP]");
        assert_eq!(filter.redact("Server: 10.0.0.255"), "Server: [IP]");
    }

    #[test]
    fn test_redact_multiple_pii_types() {
        let filter = PiiFilter::default_filter();
        let input = "User john@test.com, SSN 123-45-6789, phone (555) 123-4567";
        let expected = "User [EMAIL], SSN [SSN], phone [PHONE]";
        assert_eq!(filter.redact(input), expected);
    }

    #[test]
    fn test_redact_json() {
        let filter = PiiFilter::default_filter();
        let input = json!({
            "email": "test@example.com",
            "ssn": "123-45-6789",
            "nested": {
                "phone": "(555) 123-4567"
            },
            "array": ["a@b.com", "not-pii"],
            "number": 42
        });

        let expected = json!({
            "email": "[EMAIL]",
            "ssn": "[SSN]",
            "nested": {
                "phone": "[PHONE]"
            },
            "array": ["[EMAIL]", "not-pii"],
            "number": 42
        });

        assert_eq!(filter.redact_json(&input), expected);
    }

    #[test]
    fn test_truncation() {
        let config = PiiFilterConfig {
            max_sample_length: 20,
            ..Default::default()
        };
        let filter = PiiFilter::new(config);
        let input = "This is a very long string that should be truncated";
        let result = filter.redact(input);
        assert!(result.ends_with("...[TRUNCATED]"));
        assert!(result.len() < input.len() + 20); // Some overhead for truncation marker
    }

    #[test]
    fn test_selective_redaction() {
        let config = PiiFilterConfig {
            redact_emails: true,
            redact_ssn: false,
            redact_credit_cards: false,
            redact_phone_numbers: false,
            redact_ip_addresses: false,
            max_sample_length: 1000,
        };
        let filter = PiiFilter::new(config);

        let input = "Email: a@b.com, SSN: 123-45-6789";
        assert_eq!(filter.redact(input), "Email: [EMAIL], SSN: 123-45-6789");
    }

    #[test]
    fn test_no_pii() {
        let filter = PiiFilter::default_filter();
        let input = "This string has no PII in it";
        assert_eq!(filter.redact(input), input);
    }

    #[test]
    fn test_global_redact_function() {
        assert_eq!(
            redact_pii("Contact: test@example.com"),
            "Contact: [EMAIL]"
        );
    }
}
