//! Forms: extras-to-order (ETO) and the property inspection draft.
//!
//! Both produce a site-diary entry plus a categorised note; the form tables
//! only hold the structured state the diary note cannot. Rendering the note
//! text is therefore domain work, not presentation -- the string is persisted
//! and read back by the diary UI, so it is part of the contract.

use crate::error::{DomainError, DomainResult};
use crate::ids::JobId;

/// The next ETO number for a job, as allocated by `eto_job_sequences`.
///
/// Per-job, permanent and gap-free (domain-rules R7): the allocation *is* the
/// upsert, so two concurrent submissions cannot receive the same number and a
/// rolled-back transaction gives its number back rather than burning it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EtoNumber(pub i32);

impl EtoNumber {
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }

    /// `{job_number}-{NN}`, zero-padded to two digits.
    ///
    /// `job_number` falls back to the job id when blank, matching the legacy
    /// `String(job.job_number || jobId)`. Padding only ever *adds* characters,
    /// so a three-digit ETO stays three digits rather than being truncated.
    #[must_use]
    pub fn po_number(self, job_number: &str, job: JobId) -> String {
        let prefix = job_number.trim();
        let prefix = if prefix.is_empty() {
            job.get().to_string()
        } else {
            prefix.to_string()
        };
        format!("{prefix}-{:02}", self.0)
    }
}

/// What the client submits to raise an ETO.
#[derive(Debug, Clone)]
pub struct EtoInput {
    pub job_id: JobId,
    /// Internal justification. Explicitly not for sharing with the client.
    pub reason: String,
    /// What the extra work actually is.
    pub details: String,
}

impl EtoInput {
    pub fn validate(&self) -> DomainResult<()> {
        if self.job_id.get() <= 0 {
            return Err(DomainError::invalid("jobId", "a valid job is required"));
        }
        if self.reason.trim().is_empty() {
            return Err(DomainError::invalid(
                "reason",
                "an internal ETO reason is required",
            ));
        }
        if self.details.trim().is_empty() {
            return Err(DomainError::invalid(
                "details",
                "ETO requirements are required",
            ));
        }
        Ok(())
    }
}

/// Identifies the job in ETO and inspection note text.
#[derive(Debug, Clone, Default)]
pub struct JobHeader {
    pub job_number: String,
    pub name: String,
    pub address: String,
}

impl JobHeader {
    /// `job_number — name — address`, skipping blanks.
    #[must_use]
    pub fn line(&self) -> String {
        [&self.job_number, &self.name, &self.address]
            .into_iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" — ")
    }
}

/// The body of the `eto` diary note.
///
/// The "INTERNAL REASON" block is deliberately inside the same note as the
/// shareable part: the legacy UI relies on the heading to decide what to
/// redact when emailing, so splitting them would break that.
#[must_use]
pub fn eto_note(po_number: &str, raised_by: &str, job: &JobHeader, input: &EtoInput) -> String {
    [
        format!("ETO {po_number} — EXTRA TO ORDER"),
        format!("PURCHASE ORDER: {po_number}"),
        format!("Raised by: {raised_by}"),
        format!("Job: {}", job.line()),
        String::new(),
        "INTERNAL REASON — DO NOT EMAIL OR SHARE".to_string(),
        input.reason.trim().to_string(),
        String::new(),
        "WHAT IS REQUIRED".to_string(),
        input.details.trim().to_string(),
        String::new(),
        "Manager approval required before sharing.".to_string(),
    ]
    .join("\n")
}

/// The `work_completed` summary on the ETO's diary entry.
#[must_use]
pub fn eto_entry_summary(po_number: &str) -> String {
    format!("ETO {po_number} — Pending Manager Approval")
}

/// Pulls the PO number back out of a stored ETO note.
///
/// The number is not a column -- it only ever existed inside the note text --
/// so listing a job's ETOs means parsing it back out. Falls back to
/// `ETO-{note_id}` exactly as the legacy list did.
#[must_use]
pub fn parse_eto_number(content: &str, note_id: i32) -> String {
    content
        .lines()
        .find_map(|line| {
            let rest = line.strip_prefix("ETO ")?;
            let (number, _) = rest.split_once(" — ")?;
            let number = number.trim();
            (!number.is_empty()).then(|| number.to_string())
        })
        .unwrap_or_else(|| format!("ETO-{note_id}"))
}

/// Pulls the raiser's name out of a stored ETO note.
///
/// Only used when `action_raised_by` is null -- notes predating that column
/// carry the name in the text and nowhere else.
#[must_use]
pub fn parse_eto_raised_by(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let rest = line.strip_prefix("Raised by:")?;
        let name = rest.trim();
        (!name.is_empty()).then(|| name.to_string())
    })
}

// ── property inspection ─────────────────────────────────────────────────────

/// One row of the inspection checklist.
///
/// `client_key` is generated by the browser, not the server: it lets an offline
/// client add rows, attach photos to them and sync later without first needing
/// a server id. It is unique per form.
#[derive(Debug, Clone)]
pub struct InspectionItemInput {
    pub client_key: String,
    pub room: String,
    pub description: String,
    pub actioned: bool,
    pub sort_order: Option<i32>,
}

impl InspectionItemInput {
    /// An item counts as filled in once it has a room, a description or a
    /// photo. Empty rows are ignored rather than rejected: the client keeps a
    /// blank row at the bottom of the list for the next entry.
    #[must_use]
    pub fn has_content(&self, photos: usize) -> bool {
        !self.room.trim().is_empty() || !self.description.trim().is_empty() || photos > 0
    }
}

/// A full draft save. Absent fields clear rather than preserve, matching the
/// legacy `input.inspector ?? ""` -- the client always sends the whole form.
#[derive(Debug, Clone)]
pub struct InspectionDraftInput {
    pub inspector: String,
    pub inspection_type: String,
    pub stage: String,
    pub observations: String,
    pub weather_data: Option<serde_json::Value>,
    /// The revision the client last read. A mismatch is a 409.
    pub revision: i32,
    pub items: Vec<InspectionItemInput>,
}

impl InspectionDraftInput {
    pub fn validate(&self) -> DomainResult<()> {
        if self.revision < 0 {
            return Err(DomainError::invalid(
                "revision",
                "a valid inspection revision is required",
            ));
        }
        for item in &self.items {
            if item.client_key.trim().is_empty() {
                return Err(DomainError::invalid(
                    "items.clientKey",
                    "every inspection item needs a client key",
                ));
            }
            if item.client_key.chars().count() > 100 {
                return Err(DomainError::invalid(
                    "items.clientKey",
                    "must be 100 characters or fewer",
                ));
            }
        }
        // `inspection_form_items_client_key_unique` would reject this at the
        // database, but as a 409 rather than the 400 it really is.
        let mut keys: Vec<&str> = self.items.iter().map(|i| i.client_key.trim()).collect();
        keys.sort_unstable();
        if keys.windows(2).any(|w| w[0] == w[1]) {
            return Err(DomainError::invalid(
                "items.clientKey",
                "inspection items must have distinct client keys",
            ));
        }
        Ok(())
    }

    /// Items with a non-blank key, in the order the client sent them.
    #[must_use]
    pub fn retained(&self) -> Vec<&InspectionItemInput> {
        self.items
            .iter()
            .filter(|i| !i.client_key.trim().is_empty())
            .collect()
    }
}

/// The diary note's action status, derived from the checklist.
///
/// `None` means the note carries no action flag at all: a draft nobody has
/// typed into yet should not appear in the dashboard's outstanding-actions
/// count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionStatus {
    /// At least one filled item is still outstanding.
    Action,
    /// Every filled item is ticked off.
    Completed,
}

impl ActionStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Completed => "completed",
        }
    }
}

/// `photos` answers how many photos an item's `client_key` has.
#[must_use]
pub fn inspection_action_status(
    items: &[&InspectionItemInput],
    photos: &dyn Fn(&str) -> usize,
) -> Option<ActionStatus> {
    let filled: Vec<_> = items
        .iter()
        .filter(|i| i.has_content(photos(i.client_key.trim())))
        .collect();
    if filled.is_empty() {
        return None;
    }
    if filled.iter().any(|i| !i.actioned) {
        Some(ActionStatus::Action)
    } else {
        Some(ActionStatus::Completed)
    }
}

/// The `work_completed` summary on an inspection's diary entry.
#[must_use]
pub fn inspection_entry_summary(inspection_type: &str, stage: &str) -> String {
    let mut s = "Site Inspection".to_string();
    for part in [inspection_type.trim(), stage.trim()] {
        if !part.is_empty() {
            s.push_str(" — ");
            s.push_str(part);
        }
    }
    s
}

/// The body of the `property-inspection` diary note.
///
/// Byte-for-byte the legacy layout, emoji included: the text is stored and
/// re-read by the diary UI, so changing it would silently alter every existing
/// inspection's appearance when the row is next saved.
#[must_use]
pub fn inspection_note(
    input: &InspectionDraftInput,
    inspection_date: chrono::NaiveDate,
    photos: &dyn Fn(&str) -> usize,
) -> String {
    let items = input.retained();
    let filled: Vec<_> = items
        .into_iter()
        .filter(|i| i.has_content(photos(i.client_key.trim())))
        .collect();

    let lines: Vec<String> = filled
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let count = photos(item.client_key.trim());
            let photo_text = if count > 0 {
                format!(" [{count} photo(s)]")
            } else {
                String::new()
            };
            let room = non_blank(&item.room);
            let description = non_blank(&item.description);
            let state = if item.actioned {
                "✅ Actioned"
            } else {
                "⬜ Pending"
            };
            format!(
                "#{} | {room} | {description}{photo_text} | {state}",
                index + 1
            )
        })
        .collect();

    let details: Vec<String> = [
        ("👤 Inspector: ", input.inspector.trim()),
        ("🏷 Type: ", input.inspection_type.trim()),
        ("📍 Stage/Area: ", input.stage.trim()),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .map(|(label, v)| format!("{label}{v}"))
    .collect();

    let actioned = filled.iter().filter(|i| i.actioned).count();
    let mut out = vec![
        "📋 SITE INSPECTION".to_string(),
        format!("📅 {}", inspection_date.format("%Y-%m-%d")),
    ];
    if !details.is_empty() {
        out.push(details.join("  |  "));
    }
    out.push(String::new());
    out.push(format!(
        "INSPECTION ITEMS ({} total, {actioned} actioned):",
        filled.len()
    ));
    out.push(if lines.is_empty() {
        "Draft started — no inspection items entered yet.".to_string()
    } else {
        lines.join("\n")
    });
    let observations = input.observations.trim();
    if !observations.is_empty() {
        out.push(format!("\n📝 OBSERVATIONS:\n{observations}"));
    }
    out.join("\n")
}

/// An em dash stands in for an empty cell, so the columns stay aligned.
fn non_blank(s: &str) -> &str {
    let t = s.trim();
    if t.is_empty() {
        "—"
    } else {
        t
    }
}

/// A stale-write rejection, phrased for the person who hit it.
#[must_use]
pub fn stale_revision() -> DomainError {
    DomainError::Conflict(
        "This inspection changed in another session. Reload it before continuing.".to_string(),
    )
}

#[cfg(test)]
mod tests;
