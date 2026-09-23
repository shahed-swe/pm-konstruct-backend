//! `pmk-worker` -- the background jobs the API must not do inline.
//!
//! One job today: sweeping deleted objects out of storage. A media row is
//! removed the moment somebody presses delete, and the object it points at
//! is queued rather than deleted with it. Two reasons:
//!
//! - The delete has to be atomic with the row. Object storage is not in the
//!   database's transaction, so doing both inline means either an orphaned
//!   object (row deleted, storage call failed) or a row pointing at nothing
//!   (storage deleted, transaction rolled back).
//! - A bulk delete of forty photos would otherwise make forty round trips
//!   to storage while somebody waits.
//!
//! The queue is `media_deletion_queue`, which `pmk_app` may only insert
//! into: row-level security denies it everything else (migration 0004), so
//! the worker connects as the owner. It is the one process that is
//! deliberately cross-tenant, because an object key belongs to no tenant.
//!
//! Safe to run more than once, and safe to kill at any point: a failed
//! delete stays queued, and an object already gone counts as deleted.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use pmk_infra::config::Config;
use pmk_infra::db::{connect, PoolConfig};
use pmk_ports::repository::MediaRepository;
use pmk_ports::{ObjectStore, PortError, PortResult};

/// How many objects one pass takes.
///
/// Small enough that a pass is quick and a restart loses little, large
/// enough that a bulk delete drains in one go.
const BATCH: i64 = 50;

/// How long to wait when there was nothing to do.
const IDLE: Duration = Duration::from_secs(30);

/// How long to wait after a pass that did work, so a long backlog drains
/// without hammering storage.
const BUSY: Duration = Duration::from_secs(1);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::load()?;

    // The owner role, not `pmk_app`: the queue is denied to the application
    // role by design (migration 0004), and this process is cross-tenant.
    // `PMK_WORKER_DATABASE_URL` carries that connection; without it the
    // worker would start, find the queue empty under RLS and sweep nothing,
    // so its absence is a startup error rather than a silent no-op.
    let url = std::env::var("PMK_WORKER_DATABASE_URL").map_err(|_| {
        anyhow::anyhow!(
            "PMK_WORKER_DATABASE_URL is not set -- the sweeper needs the owner \
             role, because row-level security hides media_deletion_queue from \
             the application role"
        )
    })?;

    // Four connections is generous for one sequential loop; the pool exists
    // for reconnection, not concurrency.
    let mut pool_cfg = PoolConfig::new(url);
    pool_cfg.max_connections = 4;
    pool_cfg.min_connections = 1;
    pool_cfg.acquire_timeout = Duration::from_secs(config.database.acquire_timeout_secs);
    let pool = connect(&pool_cfg).await?;

    let media: Arc<dyn MediaRepository> =
        Arc::new(pmk_infra::repo::media::PgMediaRepository::new(pool.clone()));
    let store: Arc<dyn ObjectStore> =
        Arc::new(pmk_infra::storage::S3ObjectStore::new(&config.storage));

    tracing::info!(
        bucket = %config.storage.bucket,
        batch = BATCH,
        "media sweeper started"
    );

    // A plain loop rather than a scheduler: one job, no schedule, and a
    // cron expression would be a third place to look when it stops running.
    loop {
        let swept = match sweep(media.as_ref(), store.as_ref()).await {
            Ok(count) => count,
            Err(error) => {
                // A database or storage outage is not fatal. The queue is
                // durable, so the next pass picks up where this one stopped.
                tracing::warn!(%error, "sweep failed; retrying");
                0
            }
        };

        tokio::time::sleep(if swept == 0 { IDLE } else { BUSY }).await;
    }
}

/// The three queue operations the sweep needs.
///
/// `MediaRepository` has twenty methods and `ObjectStore` seven; narrowing to
/// what is actually called is what makes the sweep testable, since a fake for
/// either full trait would be several hundred lines of `unimplemented!()`.
/// The blanket implementations mean the real adapters satisfy these for free.
#[async_trait]
trait DeletionQueue: Send + Sync {
    async fn pending(&self, limit: i64) -> PortResult<Vec<String>>;
    async fn done(&self, key: &str) -> PortResult<()>;
    async fn failed(&self, key: &str, error: &str) -> PortResult<()>;
}

#[async_trait]
impl<T: MediaRepository + ?Sized> DeletionQueue for T {
    async fn pending(&self, limit: i64) -> PortResult<Vec<String>> {
        self.pending_deletions(limit).await
    }
    async fn done(&self, key: &str) -> PortResult<()> {
        self.mark_deleted(key).await
    }
    async fn failed(&self, key: &str, error: &str) -> PortResult<()> {
        self.mark_deletion_failed(key, error).await
    }
}

#[async_trait]
trait ObjectDeleter: Send + Sync {
    async fn delete_object(&self, key: &str) -> PortResult<()>;
}

#[async_trait]
impl<T: ObjectStore + ?Sized> ObjectDeleter for T {
    async fn delete_object(&self, key: &str) -> PortResult<()> {
        self.delete(key).await
    }
}

/// One pass. Returns how many objects were dealt with.
async fn sweep<Q, S>(media: &Q, store: &S) -> anyhow::Result<usize>
where
    Q: DeletionQueue + ?Sized,
    S: ObjectDeleter + ?Sized,
{
    let pending = media.pending(BATCH).await?;
    if pending.is_empty() {
        return Ok(0);
    }

    let mut swept = 0;
    for key in &pending {
        match store.delete_object(key).await {
            Ok(()) => {
                media.done(key).await?;
                swept += 1;
                tracing::debug!(key = %key, "swept");
            }
            // Already gone is the outcome we wanted. A retry of a partly
            // finished pass hits this, and so does a key deleted by hand.
            Err(PortError::NotFound) => {
                media.done(key).await?;
                swept += 1;
                tracing::debug!(key = %key, "already gone");
            }
            Err(error) => {
                // Recorded against the row, which carries an attempt count.
                // `pending_deletions` stops offering a key after ten
                // failures, so a permanently undeletable object does not
                // block everything behind it.
                media.failed(key, &error.to_string()).await?;
                tracing::warn!(key = %key, %error, "could not delete; will retry");
            }
        }
    }

    tracing::info!(swept, queued = pending.len(), "sweep complete");
    Ok(swept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records what the sweep did, and can be told to fail a given key.
    #[derive(Default)]
    struct FakeQueue {
        queued: Mutex<Vec<String>>,
        deleted: Mutex<Vec<String>>,
        failed: Mutex<Vec<(String, String)>>,
    }

    impl FakeQueue {
        fn with(keys: &[&str]) -> Self {
            Self {
                queued: Mutex::new(keys.iter().map(|k| (*k).to_string()).collect()),
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl DeletionQueue for FakeQueue {
        async fn pending(&self, limit: i64) -> PortResult<Vec<String>> {
            let queued = self.queued.lock().unwrap();
            let n = usize::try_from(limit)
                .unwrap_or(usize::MAX)
                .min(queued.len());
            Ok(queued[..n].to_vec())
        }
        async fn done(&self, key: &str) -> PortResult<()> {
            self.deleted.lock().unwrap().push(key.to_string());
            Ok(())
        }
        async fn failed(&self, key: &str, error: &str) -> PortResult<()> {
            self.failed
                .lock()
                .unwrap()
                .push((key.to_string(), error.to_string()));
            Ok(())
        }
    }

    struct FakeStore {
        /// Keys that are already gone from storage.
        missing: Vec<&'static str>,
        /// Keys whose delete fails outright.
        broken: Vec<&'static str>,
        seen: Mutex<Vec<String>>,
    }

    impl FakeStore {
        fn ok() -> Self {
            Self {
                missing: vec![],
                broken: vec![],
                seen: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl ObjectDeleter for FakeStore {
        async fn delete_object(&self, key: &str) -> PortResult<()> {
            self.seen.lock().unwrap().push(key.to_string());
            if self.missing.contains(&key) {
                return Err(PortError::NotFound);
            }
            if self.broken.contains(&key) {
                return Err(PortError::Storage("bucket is on fire".into()));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn an_empty_queue_does_nothing() {
        let queue = FakeQueue::default();
        let store = FakeStore::ok();
        assert_eq!(sweep(&queue, &store).await.unwrap(), 0);
        assert!(
            store.seen.lock().unwrap().is_empty(),
            "an empty queue must not reach storage at all -- it is the idle \
             case, and it runs every 30 seconds forever"
        );
    }

    #[tokio::test]
    async fn every_queued_object_is_deleted_and_marked() {
        let queue = FakeQueue::with(&["a.jpg", "b.pdf"]);
        let store = FakeStore::ok();

        assert_eq!(sweep(&queue, &store).await.unwrap(), 2);
        assert_eq!(*store.seen.lock().unwrap(), vec!["a.jpg", "b.pdf"]);
        assert_eq!(*queue.deleted.lock().unwrap(), vec!["a.jpg", "b.pdf"]);
        assert!(queue.failed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_object_already_gone_counts_as_swept() {
        // The case that matters for restarts: a pass that deleted the object
        // and died before marking the row finds it missing next time. If that
        // were recorded as a failure the key would retry ten times and then
        // sit in the queue for good.
        let queue = FakeQueue::with(&["gone.jpg"]);
        let store = FakeStore {
            missing: vec!["gone.jpg"],
            ..FakeStore::ok()
        };

        assert_eq!(sweep(&queue, &store).await.unwrap(), 1);
        assert_eq!(*queue.deleted.lock().unwrap(), vec!["gone.jpg"]);
        assert!(
            queue.failed.lock().unwrap().is_empty(),
            "a missing object is the outcome we wanted, not a failure"
        );
    }

    #[tokio::test]
    async fn one_failure_does_not_stop_the_rest() {
        let queue = FakeQueue::with(&["a.jpg", "bad.jpg", "c.jpg"]);
        let store = FakeStore {
            broken: vec!["bad.jpg"],
            ..FakeStore::ok()
        };

        assert_eq!(sweep(&queue, &store).await.unwrap(), 2);
        assert_eq!(*queue.deleted.lock().unwrap(), vec!["a.jpg", "c.jpg"]);

        let failed = queue.failed.lock().unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, "bad.jpg");
        assert!(
            failed[0].1.contains("bucket is on fire"),
            "the reason is written to the row so an operator can see why: {:?}",
            failed[0].1
        );
    }

    #[tokio::test]
    async fn a_pass_takes_at_most_one_batch() {
        let keys: Vec<String> = (0..500).map(|i| format!("{i}.jpg")).collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let queue = FakeQueue::with(&refs);
        let store = FakeStore::ok();

        let swept = sweep(&queue, &store).await.unwrap();
        assert_eq!(
            swept,
            usize::try_from(BATCH).unwrap(),
            "a bulk delete must drain over several passes, not hold one pass \
             open for 500 round trips to storage"
        );
    }
}
