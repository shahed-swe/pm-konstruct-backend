//! Queueing objects that a cascading delete is about to orphan.
//!
//! Deleting a job, a diary entry or a note removes its media rows through
//! `ON DELETE CASCADE`. The cascade happens inside Postgres, so it never runs
//! the application's delete path -- and that path is the only thing that
//! enqueues the object for the sweeper. Without these, a purged job's photos
//! stay in the bucket for ever: nothing points at them and nothing deletes
//! them, and the client who asked for the job to be removed still has their
//! site photos sitting in storage.
//!
//! Each of these runs *before* its delete, in the same transaction, because
//! after the cascade the rows are gone and there is nothing left to read.
//! Committing the delete therefore always commits the queue rows with it, and
//! rolling back drops both.
//!
//! What goes in is the **object key**, built by `pmk_domain::media::object_key`
//! -- `{company}/{entity}/{entity_id}/{stored_name}` -- and never the bare
//! `stored_name`. Storage answers a delete of a key that does not exist with
//! success, so a wrong key is not an error anybody sees: the queue drains and
//! the object stays for ever. That is exactly what migration 0015 fixes.
//!
//! The keys are built in Rust rather than assembled in SQL so that there is
//! one definition of the naming rule. Two copies would drift, and nothing
//! would fail until objects started going missing.

use sqlx::{Postgres, Row, Transaction};

use super::user::map_sqlx;
use pmk_domain::media::object_key;
use pmk_ports::PortResult;

/// Everything under a job: its own files and the media on every diary entry
/// and note that hangs off it.
pub(crate) async fn enqueue_job_media(
    tx: &mut Transaction<'_, Postgres>,
    company_id: i32,
    job_id: i32,
) -> PortResult<usize> {
    let job_rows = sqlx::query("SELECT stored_name FROM job_media WHERE job_id = $1")
        .bind(job_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(map_sqlx)?;

    let diary_rows = sqlx::query(
        "SELECT dm.stored_name, dm.diary_entry_id FROM diary_media dm \
         JOIN site_diary sd ON sd.id = dm.diary_entry_id \
         WHERE sd.job_id = $1",
    )
    .bind(job_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(map_sqlx)?;

    let mut keys = Vec::with_capacity(job_rows.len() + diary_rows.len());
    for r in &job_rows {
        let stored: String = r.get("stored_name");
        keys.push(object_key(company_id, "job", job_id, &stored));
    }
    for r in &diary_rows {
        let stored: String = r.get("stored_name");
        keys.push(object_key(
            company_id,
            "diary",
            r.get("diary_entry_id"),
            &stored,
        ));
    }
    enqueue(tx, &keys).await?;
    Ok(keys.len())
}

/// The media on one diary entry, including anything attached to its notes --
/// `diary_media.diary_entry_id` is set on those too, so one query covers both.
pub(crate) async fn enqueue_diary_entry_media(
    tx: &mut Transaction<'_, Postgres>,
    company_id: i32,
    entry_id: i32,
) -> PortResult<usize> {
    let rows = sqlx::query("SELECT stored_name FROM diary_media WHERE diary_entry_id = $1")
        .bind(entry_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(map_sqlx)?;

    let keys: Vec<String> = rows
        .iter()
        .map(|r| {
            let stored: String = r.get("stored_name");
            object_key(company_id, "diary", entry_id, &stored)
        })
        .collect();
    enqueue(tx, &keys).await?;
    Ok(keys.len())
}

/// The media attached to one note.
///
/// The key is keyed on the *entry*, not the note: a note's upload is stored
/// under its entry's prefix, with `note_id` recorded alongside.
pub(crate) async fn enqueue_note_media(
    tx: &mut Transaction<'_, Postgres>,
    company_id: i32,
    note_id: i32,
) -> PortResult<usize> {
    let rows =
        sqlx::query("SELECT stored_name, diary_entry_id FROM diary_media WHERE note_id = $1")
            .bind(note_id)
            .fetch_all(&mut **tx)
            .await
            .map_err(map_sqlx)?;

    let keys: Vec<String> = rows
        .iter()
        .map(|r| {
            let stored: String = r.get("stored_name");
            object_key(company_id, "diary", r.get("diary_entry_id"), &stored)
        })
        .collect();
    enqueue(tx, &keys).await?;
    Ok(keys.len())
}

/// `ON CONFLICT DO NOTHING` without a target: naming one would make Postgres
/// read the conflicting row, which needs SELECT on a table the application
/// role is deliberately denied (migration 0008).
async fn enqueue(tx: &mut Transaction<'_, Postgres>, keys: &[String]) -> PortResult<()> {
    if keys.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO media_deletion_queue (object_key) \
         SELECT unnest($1::text[]) ON CONFLICT DO NOTHING",
    )
    .bind(keys)
    .execute(&mut **tx)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}
