//! Binding a [`TenantScope`] to a database transaction.
//!
//! Every tenant-scoped query runs inside a transaction that has set
//! `app.company_id`. RLS policies compare against it, and with it unset they
//! evaluate false, so an unscoped query returns zero rows instead of
//! everything — fail closed. Proven by `tools/phase3/test_rls.sh`.
//!
//! `SET LOCAL` is used deliberately: it is scoped to the transaction, so a
//! pooled connection cannot leak one tenant's GUC into the next request.

use pmk_domain::tenant::TenantScope;
use sqlx::{Postgres, Transaction};

/// A transaction with the tenant GUC already applied.
pub struct ScopedTx<'t> {
    tx: Transaction<'t, Postgres>,
    scope: TenantScope,
}

impl std::fmt::Debug for ScopedTx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedTx").field("scope", &self.scope).finish_non_exhaustive()
    }
}

impl<'t> ScopedTx<'t> {
    #[must_use]
    pub const fn scope(&self) -> TenantScope { self.scope }

    /// Borrow the underlying transaction for a query.
    pub fn as_mut(&mut self) -> &mut Transaction<'t, Postgres> { &mut self.tx }

    pub async fn commit(self) -> Result<(), sqlx::Error> { self.tx.commit().await }
    pub async fn rollback(self) -> Result<(), sqlx::Error> { self.tx.rollback().await }
}

/// Opens a transaction, applies the tenant GUC, and hands it to `f`.
///
/// Commits when `f` returns `Ok`, rolls back on `Err`. Because the scope can
/// only come from an `AuthenticatedPrincipal`, there is no way to reach a
/// tenant-scoped query without a verified caller.
pub async fn with_tenant<'a, F, T, E>(
    pool: &'a sqlx::PgPool,
    scope: TenantScope,
    f: F,
) -> Result<T, E>
where
    F: for<'t> FnOnce(
        &'t mut ScopedTx<'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>> + Send + 't>>,
    E: From<sqlx::Error>,
{
    let mut tx = pool.begin().await?;

    // Bind the tenant for the life of this transaction. A bound parameter
    // cannot be used with SET, so the value is formatted -- safe because
    // CompanyId wraps an i32 and cannot carry SQL.
    sqlx::query(&format!(
        "SET LOCAL app.company_id = '{}'",
        scope.company_id().get()
    ))
    .execute(&mut *tx)
    .await?;

    let mut scoped = ScopedTx { tx, scope };
    match f(&mut scoped).await {
        Ok(v) => {
            scoped.commit().await?;
            Ok(v)
        }
        Err(e) => {
            // Rollback failure must not mask the original error.
            let _ = scoped.rollback().await;
            Err(e)
        }
    }
}
