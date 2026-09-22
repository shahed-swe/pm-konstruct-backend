//! Upload verification shared by every presigned-upload flow.
//!
//! Diary media, job media and inspection photos all end the same way: the
//! client has PUT an object to a presigned URL and now claims it is a
//! particular file. None of that claim is trusted -- the object is inspected
//! in storage before any row is written (domain-rules R10). Keeping it in one
//! place means a new upload path cannot quietly skip a check.

use pmk_domain::media::{kind_for, signature_matches, MediaKind, SIGNATURE_PROBE_BYTES};
use pmk_domain::DomainError;
use pmk_ports::ObjectStore;

use crate::{AppError, AppResult};

/// What storage actually holds, once it has been checked.
#[derive(Debug, Clone)]
pub struct VerifiedUpload {
    pub kind: MediaKind,
    pub size_bytes: i64,
}

/// Confirms the object at `key` is what the client says it is.
///
/// Rejected objects are removed rather than left in the bucket: an object
/// whose bytes disagree with its declared type has no legitimate use, and
/// leaving it would let a client stage content the type check was meant to
/// stop.
pub async fn verify(
    store: &dyn ObjectStore,
    key: &str,
    mime_type: &str,
    original_name: &str,
) -> AppResult<VerifiedUpload> {
    let kind = kind_for(mime_type).ok_or_else(|| {
        AppError::Domain(DomainError::invalid("mimeType", "is not an accepted type"))
    })?;

    let head = store
        .head(key)
        .await?
        .ok_or_else(|| AppError::Domain(DomainError::invalid("storedName", "was not uploaded")))?;

    if head.size_bytes == 0 {
        discard(store, key).await;
        return Err(AppError::Domain(DomainError::invalid(
            "size",
            "the file is empty",
        )));
    }
    if head.size_bytes > kind.max_bytes() {
        discard(store, key).await;
        return Err(AppError::Domain(DomainError::invalid(
            "size",
            "Each photo must be 20 MB or smaller, or each video must be 500 MB or smaller.",
        )));
    }

    // A ranged read: validating a 500 MB video transfers only the few bytes
    // the signature needs.
    let prefix = store.read_prefix(key, SIGNATURE_PROBE_BYTES).await?;
    if !signature_matches(kind.mime, &prefix) {
        discard(store, key).await;
        return Err(AppError::Domain(DomainError::invalid(
            "file",
            format!("File content does not match the declared type for: {original_name}"),
        )));
    }

    Ok(VerifiedUpload {
        kind,
        size_bytes: i64::try_from(head.size_bytes).unwrap_or(i64::MAX),
    })
}

/// Best-effort cleanup. A failure here is logged, not propagated: the caller's
/// error is the one the client needs to see.
pub async fn discard(store: &dyn ObjectStore, key: &str) {
    if let Err(e) = store.delete(key).await {
        tracing::warn!(key, error = %e, "could not remove a rejected upload");
    }
}
