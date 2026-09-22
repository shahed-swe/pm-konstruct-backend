//! First-run setup, company registration and password recovery.
//!
//! Everything here runs *without* a tenant scope, and for the same reason in
//! each case: there is no session yet. Registration creates the tenant, and
//! recovery is used by someone who cannot sign in. The scoping is therefore
//! in the predicates -- an email address, a token -- rather than in RLS, and
//! each query is written to touch exactly one row.
//!
//! The application role cannot read `users` or `companies` directly under RLS,
//! so the reads go through the SECURITY DEFINER functions from migration 0013.

use async_trait::async_trait;
use pmk_domain::ids::UserId;
use pmk_domain::tenant::CompanyId;
use pmk_ports::repository::{
    BootstrapRepository, CompanyRegistration, RegisteredCompany, ResetToken,
};
use pmk_ports::{PortError, PortResult};
use sqlx::{PgPool, Row};

use super::user::map_sqlx;

#[derive(Debug, Clone)]
pub struct PgBootstrapRepository {
    pool: PgPool,
}

impl PgBootstrapRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BootstrapRepository for PgBootstrapRepository {
    async fn any_users_exist(&self) -> PortResult<bool> {
        let exists: bool = sqlx::query_scalar("SELECT auth_any_users_exist()")
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(exists)
    }

    async fn register_company(
        &self,
        input: &CompanyRegistration,
        password_hash: &str,
        bootstrap: bool,
    ) -> PortResult<RegisteredCompany> {
        // SERIALIZABLE, set here rather than inside the function: by the
        // time a function body runs a query is already in flight and SET
        // TRANSACTION is refused. Two people registering at the same moment
        // cannot both see an empty database and both create a company, and
        // the duplicate checks cannot pass for both before either commits.
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;

        let row = sqlx::query(
            "SELECT company_id, user_id, name, email, role, phone, active \
             FROM auth_register_company($1,$2,$3,$4,$5,$6)",
        )
        .bind(input.company_name.trim())
        .bind(input.manager.name.trim())
        .bind(input.manager.normalised_email())
        .bind(password_hash)
        .bind(input.manager.phone.as_deref())
        .bind(bootstrap)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_register_error)?;

        // Read out before the commit, which consumes the transaction.
        let registered = RegisteredCompany {
            company_id: CompanyId::new(row.get("company_id")),
            manager: pmk_domain::User {
                id: UserId(row.get("user_id")),
                company_id: CompanyId::new(row.get("company_id")),
                name: row.get("name"),
                email: row.get("email"),
                role: pmk_domain::access::Role::parse(row.get("role"))
                    .unwrap_or(pmk_domain::access::Role::Manager),
                phone: row.get("phone"),
                active: row.get("active"),
                password_changed_at: None,
            },
        };

        // A serialization failure surfaces here rather than at the statement,
        // so the commit is mapped too.
        tx.commit().await.map_err(map_register_error)?;
        Ok(registered)
    }

    async fn recovery_context(&self, email: &str) -> PortResult<Option<(UserId, i64)>> {
        let row = sqlx::query("SELECT user_id, active_users FROM auth_recovery_context($1)")
            .bind(email.trim().to_lowercase())
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(row.map(|r| (UserId(r.get("user_id")), r.get("active_users"))))
    }

    async fn store_reset_token(
        &self,
        user: UserId,
        token: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> PortResult<()> {
        sqlx::query("SELECT auth_store_reset_token($1,$2,$3)")
            .bind(user.get())
            .bind(token)
            .bind(expires_at)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn find_reset_token(
        &self,
        raw: &str,
        hashed_code: &str,
    ) -> PortResult<Option<ResetToken>> {
        let row = sqlx::query("SELECT id, user_id FROM auth_find_reset_token($1,$2)")
            .bind(raw)
            .bind(hashed_code)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(row.map(|r| ResetToken {
            id: r.get("id"),
            user_id: UserId(r.get("user_id")),
        }))
    }

    async fn claim_reset_and_set_password(
        &self,
        token_id: i32,
        user: UserId,
        password_hash: &str,
    ) -> PortResult<bool> {
        let claimed: bool = sqlx::query_scalar("SELECT auth_claim_reset($1,$2,$3)")
            .bind(token_id)
            .bind(user.get())
            .bind(password_hash)
            .fetch_one(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(claimed)
    }
}

/// Turns the registration function's signalled errors into port errors.
///
/// The function raises with explicit SQLSTATEs so the reasons stay
/// distinguishable without parsing message text.
fn map_register_error(e: sqlx::Error) -> PortError {
    if let sqlx::Error::Database(db) = &e {
        match db.code().as_deref() {
            // A user or company with that name already exists. Mapped to a
            // stable name rather than passing the database's message through,
            // so the wording the client sees lives with the other messages
            // instead of in a migration.
            Some("23505") => {
                let constraint = if db.message().contains("company") {
                    "registration_company_taken"
                } else {
                    "registration_email_taken"
                };
                return PortError::Conflict {
                    constraint: Some(constraint.to_string()),
                };
            }
            // Setup has already been completed.
            Some("P0001") => {
                return PortError::Rejected {
                    service: "setup",
                    detail: db.message().to_string(),
                }
            }
            // Serialization failure or deadlock: someone else got there first.
            Some("40001" | "40P01") => {
                return PortError::Conflict {
                    constraint: Some("registration_in_progress".into()),
                }
            }
            _ => {}
        }
    }
    map_sqlx(e)
}
