//! `EventBus` over Postgres `LISTEN`/`NOTIFY`.
//!
//! Every API instance LISTENs on one channel and forwards what arrives into a
//! local broadcast channel, which the SSE handlers read. Publishing is a
//! `pg_notify`, so an event raised on any instance reaches the clients
//! connected to all of them -- the thing the legacy in-process broker could
//! not do.
//!
//! The listener holds its own connection rather than borrowing one from the
//! pool: a LISTEN lasts for the life of the session, and a pooled connection
//! would take the subscription away with it when it was returned.

use async_trait::async_trait;
use pmk_ports::{BroadcastEvent, EventBus, PortError, PortResult, MAX_EVENT_BYTES};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::broadcast;

/// The NOTIFY channel every instance shares.
const CHANNEL: &str = "pmk_events";

/// How many events a slow subscriber may fall behind before it starts losing
/// them.
///
/// Dropping is the right failure here: a browser that has stopped reading must
/// not hold up everyone else, and the UI refetches on reconnect anyway.
const BUFFER: usize = 256;

pub struct PgEventBus {
    pool: PgPool,
    tx: broadcast::Sender<BroadcastEvent>,
}

impl std::fmt::Debug for PgEventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgEventBus")
            .field("subscribers", &self.tx.receiver_count())
            .finish_non_exhaustive()
    }
}

impl PgEventBus {
    /// Starts listening. The background task ends when the bus is dropped.
    ///
    /// # Errors
    /// If the initial LISTEN connection cannot be established.
    pub async fn start(pool: PgPool) -> PortResult<Self> {
        let (tx, _) = broadcast::channel(BUFFER);

        let mut listener =
            PgListener::connect_with(&pool)
                .await
                .map_err(|e| PortError::Unavailable {
                    service: "events",
                    detail: e.to_string(),
                })?;
        listener
            .listen(CHANNEL)
            .await
            .map_err(|e| PortError::Unavailable {
                service: "events",
                detail: e.to_string(),
            })?;

        let forward = tx.clone();
        tokio::spawn(async move {
            loop {
                match listener.recv().await {
                    Ok(note) => match serde_json::from_str::<BroadcastEvent>(note.payload()) {
                        Ok(event) => {
                            // `send` fails only when nobody is subscribed,
                            // which is the normal state of an idle server.
                            let _ = forward.send(event);
                        }
                        Err(e) => tracing::warn!(error = %e, "dropped an unreadable event"),
                    },
                    Err(e) => {
                        // sqlx reconnects the listener itself; this is logged
                        // so a flapping connection is visible rather than
                        // silently costing events.
                        tracing::warn!(error = %e, "event listener interrupted");
                    }
                }
            }
        });

        Ok(Self { pool, tx })
    }
}

#[async_trait]
impl EventBus for PgEventBus {
    async fn publish(&self, event: &BroadcastEvent) -> PortResult<()> {
        let payload = serde_json::to_string(event)
            .map_err(|e| PortError::Storage(format!("could not encode an event: {e}")))?;

        // Postgres refuses a NOTIFY payload over 8000 bytes. Failing here
        // turns "the client silently never heard about it" into something the
        // logs show.
        if payload.len() > MAX_EVENT_BYTES {
            tracing::warn!(
                kind = %event.kind,
                bytes = payload.len(),
                "event too large to broadcast; clients will refetch instead"
            );
            return Ok(());
        }

        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(CHANNEL)
            .bind(&payload)
            .execute(&self.pool)
            .await
            .map_err(|e| PortError::Unavailable {
                service: "events",
                detail: e.to_string(),
            })?;
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<BroadcastEvent> {
        self.tx.subscribe()
    }
}
