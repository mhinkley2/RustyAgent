//! Whether a failed provider call is worth attempting again.
//!
//! [`ApiError`] already classifies failures precisely, and the rate-limit
//! variant carries the server's own backoff. The runtime used to format all of
//! that into a string one line before it would have been useful. This module
//! is the classification that string was hiding.
//!
//! The rule throughout: retry only on evidence that the failure was transient.
//! An unknown error is not evidence — it is the absence of it — so anything
//! unrecognised is permanent. Retrying a deterministic failure doubles the
//! spend and the wall-clock to reach an identical outcome.

use std::time::Duration;

use crate::error::ApiError;

/// What a failed call says about trying again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Worth another attempt.
    ///
    /// `wait` is the provider's *own* instruction, when it gave one. A caller
    /// that has been told how long to wait should wait that long and not
    /// layer an invented backoff on top of a server that has already answered
    /// the question.
    Transient { wait: Option<Duration> },
    /// A second attempt reproduces this exactly.
    Permanent,
}

impl FailureKind {
    pub fn is_transient(self) -> bool {
        matches!(self, Self::Transient { .. })
    }

    /// The provider's requested delay, if it supplied one.
    pub fn wait(self) -> Option<Duration> {
        match self {
            Self::Transient { wait } => wait,
            Self::Permanent => None,
        }
    }
}

/// HTTP statuses worth retrying.
///
/// `408` is the server saying the request timed out on its side, `429` is a
/// rate limit that arrived without the header that would have made it a
/// [`ApiError::RateLimited`], and 5xx is the server admitting the fault. Every
/// other 4xx — `400`, `401`, `403`, `404` — is a configuration or contract
/// error that a second identical request reproduces exactly.
fn http_status_is_transient(status: u16) -> bool {
    status == 408 || status == 429 || (500..600).contains(&status)
}

/// Decide whether `error` is worth another attempt.
pub fn classify(error: &ApiError) -> FailureKind {
    match error {
        // The provider stated the delay; honour it rather than guessing.
        ApiError::RateLimited { retry_after_secs } => FailureKind::Transient {
            wait: Some(Duration::from_secs(*retry_after_secs)),
        },

        // The request never reached a server, or the connection died carrying
        // the answer back. Neither says anything about the request itself.
        ApiError::Network(_) | ApiError::StreamEnded => FailureKind::Transient { wait: None },

        // The variant alone decides nothing here — a matcher that stopped at
        // `ApiError::Http` would retry a 401 until the budget ran out.
        ApiError::Http { status, .. } => {
            if http_status_is_transient(*status) {
                FailureKind::Transient { wait: None }
            } else {
                FailureKind::Permanent
            }
        }

        // Our bug or a provider contract change; the same request fails the
        // same way.
        ApiError::Serialization(_) => FailureKind::Permanent,

        // Not having a key is not a transient condition.
        ApiError::Keychain(_) => FailureKind::Permanent,

        // Opaque by construction. Absence of evidence, so no retry.
        ApiError::Provider(_) => FailureKind::Permanent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_limit_is_retried_after_the_delay_the_provider_asked_for() {
        let kind = classify(&ApiError::RateLimited { retry_after_secs: 30 });

        assert_eq!(kind, FailureKind::Transient { wait: Some(Duration::from_secs(30)) });
        assert_eq!(kind.wait(), Some(Duration::from_secs(30)));
    }

    #[test]
    fn a_dropped_stream_is_retried_without_a_stated_delay() {
        let kind = classify(&ApiError::StreamEnded);

        assert_eq!(kind, FailureKind::Transient { wait: None });
        assert!(kind.is_transient());
        assert_eq!(kind.wait(), None, "the caller picks the backoff");
    }

    #[test]
    fn a_server_side_failure_is_retried() {
        for status in [500, 502, 503, 504] {
            let kind = classify(&ApiError::Http { status, body: "boom".into() });
            assert!(kind.is_transient(), "HTTP {status} should be retried");
        }
    }

    #[test]
    fn a_timeout_or_a_bare_rate_limit_is_retried() {
        for status in [408, 429] {
            let kind = classify(&ApiError::Http { status, body: String::new() });
            assert!(kind.is_transient(), "HTTP {status} should be retried");
        }
    }

    /// The case a match on the variant alone gets wrong: these are `Http` too,
    /// and retrying them burns the whole budget to be refused identically.
    #[test]
    fn a_client_error_is_never_retried() {
        for status in [400, 401, 403, 404, 422] {
            let kind = classify(&ApiError::Http { status, body: "nope".into() });
            assert_eq!(kind, FailureKind::Permanent, "HTTP {status} must not be retried");
        }
    }

    #[test]
    fn a_missing_key_is_not_a_transient_condition() {
        assert_eq!(
            classify(&ApiError::Keychain("no key".into())),
            FailureKind::Permanent
        );
    }

    #[test]
    fn a_serialization_failure_is_our_bug_not_the_networks() {
        let err: serde_json::Error = serde_json::from_str::<i32>("{").unwrap_err();

        assert_eq!(classify(&ApiError::Serialization(err)), FailureKind::Permanent);
    }

    /// An unknown error is not evidence of a transient one.
    #[test]
    fn an_opaque_provider_error_is_not_retried() {
        assert_eq!(
            classify(&ApiError::Provider("something went wrong".into())),
            FailureKind::Permanent
        );
    }

    #[test]
    fn permanent_failures_never_offer_a_delay() {
        assert_eq!(FailureKind::Permanent.wait(), None);
        assert!(!FailureKind::Permanent.is_transient());
    }
}
