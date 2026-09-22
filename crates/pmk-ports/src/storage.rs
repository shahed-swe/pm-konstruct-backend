//! Object storage port.
//!
//! The legacy app talked to Replit's storage sidecar on `127.0.0.1:1106`
//! (Analysis 4.6). This replaces it with an S3-compatible interface, backed by
//! MinIO in every environment so local, staging and production exercise the
//! same code path.
//!
//! Uploads are **presigned and direct**: the browser PUTs to storage, the API
//! never buffers the bytes. That removes 500 MB videos from the API process
//! entirely -- the legacy server wrote them to instance-local disk first, which
//! is also why it could not run more than one instance.

use async_trait::async_trait;

use crate::PortResult;

/// A time-limited URL the client uses directly.
#[derive(Debug, Clone)]
pub struct PresignedUrl {
    pub url: String,
    pub expires_in_secs: u64,
}

/// What is known about a stored object.
#[derive(Debug, Clone)]
pub struct ObjectHead {
    pub size_bytes: u64,
    pub content_type: Option<String>,
}

#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// A URL the client may PUT to, for exactly this key and content type.
    ///
    /// The content type is bound into the signature so a client cannot presign
    /// a JPEG and then upload something else.
    async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        max_bytes: u64,
    ) -> PortResult<PresignedUrl>;

    /// A URL the client may GET from. Short-lived: these end up in `<img>`
    /// tags and browser history.
    async fn presign_get(&self, key: &str) -> PortResult<PresignedUrl>;

    /// Confirms an object exists and reports its size, so the API can verify
    /// what was actually uploaded rather than trusting the client's claim.
    async fn head(&self, key: &str) -> PortResult<Option<ObjectHead>>;

    /// Reads the first `n` bytes, for magic-byte validation after upload.
    async fn read_prefix(&self, key: &str, n: usize) -> PortResult<Vec<u8>>;

    async fn delete(&self, key: &str) -> PortResult<()>;

    /// Used by `/readyz`: a storage outage should surface as not-ready rather
    /// than as a failed upload halfway through someone's day.
    async fn health_check(&self) -> PortResult<()>;
}
