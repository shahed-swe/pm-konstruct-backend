//! Typed identifiers.
//!
//! The legacy code passes bare `number` everywhere, which is how
//! `syncPrimarySupervisor` came to read an assignment id as a user id
//! (domain-rules R4). These newtypes make that class of mistake a type error.

use std::fmt;

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
                 serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub i32);

        impl $name {
            #[must_use]
            pub const fn new(v: i32) -> Self { Self(v) }
            #[must_use]
            pub const fn get(self) -> i32 { self.0 }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<$name> for i32 {
            fn from(v: $name) -> i32 { v.0 }
        }
    };
}

id_type!(
    /// A user. Distinct from `JobAssignmentId` -- see domain-rules R4.
    UserId
);
id_type!(JobId);
id_type!(
    /// A row in `job_assignments`, NOT the assigned user.
    JobAssignmentId
);
id_type!(DiaryEntryId);
id_type!(DiaryNoteId);
id_type!(DiaryNoteCommentId);
id_type!(MediaId);
id_type!(CallForwardItemId);
id_type!(CallForwardTemplateId);
id_type!(SchedulerWorkerId);
id_type!(SchedulerAllocationId);
id_type!(SchedulerAbsenceId);
id_type!(InspectionFormId);
id_type!(InspectionFormItemId);
id_type!(JobTaskId);
id_type!(NotificationId);
id_type!(ProgressId);
id_type!(ReportId);
