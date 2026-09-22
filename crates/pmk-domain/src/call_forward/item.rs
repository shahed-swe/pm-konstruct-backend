//! Call-forward items — domain-rules R2.

use crate::call_forward::{CfStatus, DelayInfo};
use crate::ids::{CallForwardItemId, JobId};
use crate::{DomainError, DomainResult};

/// `HEADER` groups, `STAGE_CLAIM` is a billable milestone, `TASK` is work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ItemType {
    #[serde(rename = "HEADER")]
    Header,
    #[serde(rename = "STAGE_CLAIM")]
    StageClaim,
    #[serde(rename = "TASK")]
    Task,
}

impl ItemType {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "HEADER" => Some(Self::Header),
            "STAGE_CLAIM" => Some(Self::StageClaim),
            "TASK" => Some(Self::Task),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Header => "HEADER",
            Self::StageClaim => "STAGE_CLAIM",
            Self::Task => "TASK",
        }
    }

    /// Only headers may contain children. Enforced here because a CHECK
    /// constraint cannot see the parent row.
    #[must_use]
    pub const fn can_parent(self) -> bool {
        matches!(self, Self::Header)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallForwardItem {
    pub id: CallForwardItemId,
    pub job_id: JobId,
    pub title: String,
    pub item_type: ItemType,
    pub supplier_trade: Option<String>,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_start: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub status: Option<CfStatus>,
    pub notes: Option<String>,
    pub sort_order: i32,
    pub parent_id: Option<CallForwardItemId>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// An item plus its computed delay (R1). `delayStatus` is never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallForwardItemWithDelay {
    pub item: CallForwardItem,
    pub delay: DelayInfo,
}

impl CallForwardItem {
    #[must_use]
    pub fn with_delay(self, today: chrono::NaiveDate) -> CallForwardItemWithDelay {
        let delay = crate::call_forward::compute_delay_info(
            &crate::call_forward::DelayInput {
                status: self.status,
                est_start: self.est_start,
                est_finish: self.est_finish,
                actual_start: self.actual_start,
                actual_finish: self.actual_finish,
            },
            today,
        );
        CallForwardItemWithDelay { item: self, delay }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CallForwardInput {
    pub title: String,
    pub item_type: Option<ItemType>,
    pub supplier_trade: Option<String>,
    pub est_start: Option<chrono::NaiveDate>,
    pub est_finish: Option<chrono::NaiveDate>,
    pub actual_start: Option<chrono::NaiveDate>,
    pub actual_finish: Option<chrono::NaiveDate>,
    pub status: Option<CfStatus>,
    pub notes: Option<String>,
    pub sort_order: Option<i32>,
    pub parent_id: Option<i32>,
}

impl CallForwardInput {
    pub fn validate(&self) -> DomainResult<()> {
        if self.title.trim().is_empty() {
            return Err(DomainError::invalid("title", "is required"));
        }
        if self.title.len() > 500 {
            return Err(DomainError::invalid(
                "title",
                "must be 500 characters or fewer",
            ));
        }
        if let Some(t) = &self.supplier_trade {
            if t.len() > 150 {
                return Err(DomainError::invalid(
                    "supplierTrade",
                    "must be 150 characters or fewer",
                ));
            }
        }
        // Not a database constraint, but an inverted estimate is always a typo
        // and it would silently produce a nonsensical delay count (R1).
        if let (Some(s), Some(f)) = (self.est_start, self.est_finish) {
            if s > f {
                return Err(DomainError::invalid(
                    "estFinish",
                    "must be on or after the estimated start",
                ));
            }
        }
        if let (Some(s), Some(f)) = (self.actual_start, self.actual_finish) {
            if s > f {
                return Err(DomainError::invalid(
                    "actualFinish",
                    "must be on or after the actual start",
                ));
            }
        }
        Ok(())
    }
}

/// Validates a proposed parent/child link (R2).
///
/// The legacy schema had no foreign key on `parent_id` at all, so an item could
/// point at a deleted row, at an item on another job, or at itself. Migration
/// 0003 added the key and a self-reference check; the rest is here because a
/// CHECK constraint cannot read another row.
pub fn validate_parent(
    child_type: ItemType,
    child_id: Option<CallForwardItemId>,
    parent: Option<(CallForwardItemId, ItemType, JobId)>,
    child_job: JobId,
) -> DomainResult<()> {
    let Some((parent_id, parent_type, parent_job)) = parent else {
        return Ok(());
    };
    if Some(parent_id) == child_id {
        return Err(DomainError::invalid(
            "parentId",
            "an item cannot be its own parent",
        ));
    }
    if parent_job != child_job {
        return Err(DomainError::invalid(
            "parentId",
            "must be an item on the same job",
        ));
    }
    if !parent_type.can_parent() {
        return Err(DomainError::invalid(
            "parentId",
            "only a HEADER can contain other items",
        ));
    }
    if child_type == ItemType::Header {
        return Err(DomainError::invalid(
            "parentId",
            "a HEADER cannot be nested inside another item",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> CallForwardInput {
        CallForwardInput {
            title: "Frame walls".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_minimal_item_validates() {
        assert!(base().validate().is_ok());
    }

    #[test]
    fn title_is_required() {
        let mut i = base();
        i.title = "  ".into();
        assert!(i.validate().is_err());
    }

    #[test]
    fn inverted_estimates_are_rejected() {
        let mut i = base();
        i.est_start = Some("2026-05-10".parse().unwrap());
        i.est_finish = Some("2026-05-01".parse().unwrap());
        match i.validate() {
            Err(DomainError::Invalid { field, .. }) => assert_eq!(field, "estFinish"),
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn equal_estimate_dates_are_a_one_day_task() {
        let mut i = base();
        let d = "2026-05-10".parse().unwrap();
        i.est_start = Some(d);
        i.est_finish = Some(d);
        assert!(i.validate().is_ok());
    }

    #[test]
    fn item_types_round_trip() {
        for s in ["HEADER", "STAGE_CLAIM", "TASK"] {
            assert_eq!(ItemType::parse(s).map(ItemType::as_str), Some(s));
        }
        assert_eq!(ItemType::parse("MILESTONE"), None);
    }

    #[test]
    fn only_headers_may_parent() {
        assert!(ItemType::Header.can_parent());
        assert!(!ItemType::Task.can_parent());
        assert!(!ItemType::StageClaim.can_parent());
    }

    #[test]
    fn a_task_under_a_header_is_allowed() {
        let r = validate_parent(
            ItemType::Task,
            Some(CallForwardItemId(2)),
            Some((CallForwardItemId(1), ItemType::Header, JobId(1))),
            JobId(1),
        );
        assert!(r.is_ok());
    }

    #[test]
    fn a_task_cannot_parent_another_task() {
        let r = validate_parent(
            ItemType::Task,
            Some(CallForwardItemId(2)),
            Some((CallForwardItemId(1), ItemType::Task, JobId(1))),
            JobId(1),
        );
        assert!(r.is_err(), "only HEADER can contain items");
    }

    #[test]
    fn a_header_cannot_be_nested() {
        let r = validate_parent(
            ItemType::Header,
            Some(CallForwardItemId(2)),
            Some((CallForwardItemId(1), ItemType::Header, JobId(1))),
            JobId(1),
        );
        assert!(r.is_err());
    }

    #[test]
    fn a_parent_on_another_job_is_rejected() {
        let r = validate_parent(
            ItemType::Task,
            Some(CallForwardItemId(2)),
            Some((CallForwardItemId(1), ItemType::Header, JobId(99))),
            JobId(1),
        );
        assert!(r.is_err(), "cross-job parenting must not be possible");
    }

    #[test]
    fn self_parenting_is_rejected() {
        let r = validate_parent(
            ItemType::Task,
            Some(CallForwardItemId(1)),
            Some((CallForwardItemId(1), ItemType::Header, JobId(1))),
            JobId(1),
        );
        assert!(r.is_err());
    }

    #[test]
    fn no_parent_is_always_fine() {
        assert!(validate_parent(ItemType::Header, None, None, JobId(1)).is_ok());
    }

    #[test]
    fn delay_is_computed_not_stored() {
        use crate::call_forward::DelayStatus;
        let item = CallForwardItem {
            id: CallForwardItemId(1),
            job_id: JobId(1),
            title: "t".into(),
            item_type: ItemType::Task,
            supplier_trade: None,
            est_start: None,
            est_finish: Some("2026-09-16".parse().unwrap()),
            actual_start: None,
            actual_finish: None,
            status: Some(CfStatus::NotStarted),
            notes: None,
            sort_order: 0,
            parent_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let out = item.with_delay("2026-09-21".parse().unwrap());
        assert_eq!(out.delay.delay_status, DelayStatus::Delayed);
        assert_eq!(out.delay.delay_days, Some(5));
    }
}
