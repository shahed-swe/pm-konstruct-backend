//! Typed extractors.
//!
//! The legacy stack stacked two or more middlewares per route, each of which
//! re-verified the token and re-queried the user -- four to six round-trips per
//! request (Analysis 4.5). Here `AuthUser` resolves the session once and later
//! extractors reuse it.
//!
//! The types also make authorisation non-optional: a handler that needs a
//! tenant takes `Entitled`, and one that needs a permission takes
//! `RequirePermission<..>`. Forgetting is a compile error, not a silent hole.

pub mod auth;

pub use auth::{
    AuthUser, CallForwardRead, CallForwardWrite, Entitled, JobsRead, JobsWrite, ManagerOnly,
    ManagerOrSupervisor, MaybeAuth, PermissionSpec, ReportsRead, RequirePermission, RequireRole,
    SiteDiaryRead, SiteDiaryWrite, TradeSchedulerRead, TradeSchedulerWrite,
};
