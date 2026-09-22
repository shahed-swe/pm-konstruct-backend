//! S3-compatible object store, targeting MinIO.
//!
//! MinIO community edition speaks the S3 API unrestricted, which is all this
//! uses -- the Console it stripped down in 2025 is irrelevant here. Because the
//! adapter is plain S3, swapping to AWS S3, Cloudflare R2, Garage or SeaweedFS
//! is a configuration change rather than a code change.

use async_trait::async_trait;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::Client;
use pmk_ports::{ObjectHead, ObjectStore, PortError, PortResult, PresignedUrl};
use std::time::Duration;

use crate::config::StorageConfig;

#[derive(Clone)]
pub struct S3ObjectStore {
    /// Used for direct calls from the API.
    client: Client,
    /// Used only to generate presigned URLs, so they carry the host the
    /// *client* can reach rather than the one the API uses.
    presign_client: Client,
    bucket: String,
    presign_ttl: Duration,
}

impl std::fmt::Debug for S3ObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3ObjectStore")
            .field("bucket", &self.bucket)
            .finish_non_exhaustive()
    }
}

impl S3ObjectStore {
    #[must_use]
    pub fn new(cfg: &StorageConfig) -> Self {
        let creds = Credentials::new(
            &cfg.access_key_id,
            &cfg.secret_access_key,
            None,
            None,
            "pmk-config",
        );

        // MinIO addresses buckets by path, not by subdomain. Without
        // force_path_style the SDK tries `http://pmk-media.localhost:9000`,
        // which does not resolve.
        let build = |endpoint: &str| {
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .region(Region::new(cfg.region.clone()))
                .endpoint_url(endpoint)
                .credentials_provider(creds.clone())
                .force_path_style(cfg.force_path_style)
                .build()
        };

        let public = cfg.public_endpoint.as_deref().unwrap_or(&cfg.endpoint);

        Self {
            client: Client::from_conf(build(&cfg.endpoint)),
            presign_client: Client::from_conf(build(public)),
            bucket: cfg.bucket.clone(),
            presign_ttl: Duration::from_secs(cfg.presign_ttl_secs),
        }
    }

    fn presign_config(&self) -> PortResult<PresigningConfig> {
        PresigningConfig::expires_in(self.presign_ttl).map_err(|e| PortError::Unavailable {
            service: "storage",
            detail: e.to_string(),
        })
    }

    fn unavailable(e: impl std::fmt::Display) -> PortError {
        PortError::Unavailable {
            service: "storage",
            detail: e.to_string(),
        }
    }
}

#[async_trait]
impl ObjectStore for S3ObjectStore {
    async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        max_bytes: u64,
    ) -> PortResult<PresignedUrl> {
        // `content_type` and `content_length` are signed into the URL, so the
        // client cannot presign a small JPEG and then upload a large
        // executable: S3 rejects the PUT if either header differs.
        let req = self
            .presign_client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .content_length(
                i64::try_from(max_bytes).map_err(|_| PortError::Storage("size overflow".into()))?,
            )
            .presigned(self.presign_config()?)
            .await
            .map_err(Self::unavailable)?;

        Ok(PresignedUrl {
            url: req.uri().to_string(),
            expires_in_secs: self.presign_ttl.as_secs(),
        })
    }

    async fn presign_get(&self, key: &str) -> PortResult<PresignedUrl> {
        let req = self
            .presign_client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(self.presign_config()?)
            .await
            .map_err(Self::unavailable)?;

        Ok(PresignedUrl {
            url: req.uri().to_string(),
            expires_in_secs: self.presign_ttl.as_secs(),
        })
    }

    async fn head(&self, key: &str) -> PortResult<Option<ObjectHead>> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(out) => Ok(Some(ObjectHead {
                size_bytes: u64::try_from(out.content_length().unwrap_or(0)).unwrap_or(0),
                content_type: out.content_type().map(ToString::to_string),
            })),
            Err(e) => {
                // A missing object is an expected answer, not a failure: it is
                // how "did the client actually upload?" is asked.
                if e.raw_response().is_some_and(|r| r.status().as_u16() == 404) {
                    Ok(None)
                } else {
                    Err(Self::unavailable(e))
                }
            }
        }
    }

    async fn read_prefix(&self, key: &str, n: usize) -> PortResult<Vec<u8>> {
        // A ranged GET, so validating a 500 MB video still only transfers the
        // handful of bytes the signature check needs.
        let end = n.saturating_sub(1);
        let out = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .range(format!("bytes=0-{end}"))
            .send()
            .await
            .map_err(Self::unavailable)?;

        let data = out
            .body
            .collect()
            .await
            .map_err(|e| PortError::Storage(e.to_string()))?;
        Ok(data.into_bytes().to_vec())
    }

    async fn delete(&self, key: &str) -> PortResult<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(Self::unavailable)?;
        Ok(())
    }

    async fn health_check(&self) -> PortResult<()> {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(Self::unavailable)?;
        Ok(())
    }
}

pub use pmk_domain::media::object_key;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_tenant_prefixed() {
        let k = object_key(7, "diary", 42, "abc.jpg");
        assert_eq!(k, "7/diary/42/abc.jpg");
        assert!(
            k.starts_with("7/"),
            "a bucket policy can scope on this prefix"
        );
    }

    #[test]
    fn different_tenants_never_share_a_prefix() {
        let a = object_key(1, "diary", 1, "x.jpg");
        let b = object_key(2, "diary", 1, "x.jpg");
        assert_ne!(a, b);
    }
}
