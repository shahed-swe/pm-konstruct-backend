//! `UserRepository` over Postgres.

use async_trait::async_trait;
use pmk_domain::access::{Permission, Role};
use pmk_domain::ids::UserId;
use pmk_domain::tenant::{CompanyId, TenantScope};
use pmk_domain::User;
use pmk_ports::repository::{StoredCredential, UserRepository};
use pmk_ports::{PortError, PortResult};
use sqlx::PgPool;

use crate::db::tenant::set_tenant;

#[derive(Debug, sqlx::FromRow)]
struct UserRow {
    id: i32,
    company_id: i32,
    name: String,
    email: String,
    role: String,
    phone: Option<String>,
    active: bool,
    password_changed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl UserRow {
    /// A role outside the enum means the `users_role_check` constraint was
    /// bypassed, which is corrupt data rather than a user error.
    fn into_domain(self) -> PortResult<User> {
        let role = Role::parse(&self.role).ok_or_else(|| {
            PortError::Storage(format!(
                "user {} has unrecognised role '{}'",
                self.id, self.role
            ))
        })?;
        Ok(User {
            id: UserId(self.id),
            company_id: CompanyId(self.company_id),
            name: self.name,
            email: self.email,
            role,
            phone: self.phone,
            active: self.active,
            password_changed_at: self.password_changed_at,
        })
    }
}

const USER_COLUMNS: &str = "id, company_id, name, email, role, phone, active, password_changed_at";

#[derive(Debug, Clone)]
pub struct PgUserRepository {
    pool: PgPool,
}

impl PgUserRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserRepository for PgUserRepository {
    async fn find_credential_by_email(&self, email: &str) -> PortResult<Option<StoredCredential>> {
        // Login runs before any tenant is known -- the tenant is derived from
        // the row being read -- but `users` has FORCE RLS. Rather than giving
        // the app role BYPASSRLS, this goes through a narrowly-scoped
        // SECURITY DEFINER function (migration 0006) that returns at most one
        // row and touches no other table.
        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            user: UserRow,
            password_hash: Option<String>,
        }

        let row: Option<Row> = sqlx::query_as(
            "SELECT id, company_id, name, email, role, phone, active, \
             password_changed_at, password_hash \
             FROM auth_find_credential_by_email($1)",
        )
        .bind(User::normalise_email(email))
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        row.map(|r| {
            Ok(StoredCredential {
                user: r.user.into_domain()?,
                password_hash: r.password_hash,
            })
        })
        .transpose()
    }

    async fn find_by_id_unscoped(&self, id: UserId) -> PortResult<Option<User>> {
        // Same pre-tenant constraint as the login lookup; see 0006.
        let row: Option<UserRow> = sqlx::query_as(
            "SELECT id, company_id, name, email, role, phone, active, password_changed_at \
             FROM auth_find_user_by_id($1)",
        )
        .bind(id.get())
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        row.map(UserRow::into_domain).transpose()
    }

    async fn replace_password_hash(&self, id: UserId, hash: &str) -> PortResult<()> {
        // `password_changed_at` moves in the same statement as the hash, which
        // is what invalidates already-issued access tokens. Runs pre-tenant
        // during the login rehash, so it uses the 0006 function.
        sqlx::query("SELECT auth_replace_password_hash($2, $1)")
            .bind(hash)
            .bind(id.get())
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn permissions_for(&self, scope: TenantScope, id: UserId) -> PortResult<Vec<Permission>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;

        #[derive(sqlx::FromRow)]
        struct Row {
            resource: String,
            action: String,
        }

        let rows: Vec<Row> =
            sqlx::query_as("SELECT resource, action FROM user_permissions WHERE user_id = $1")
                .bind(id.get())
                .fetch_all(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|r| Permission::new(r.resource, r.action))
            .collect())
    }

    async fn list(&self, scope: TenantScope) -> PortResult<Vec<User>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        let sql = format!("SELECT {USER_COLUMNS} FROM users ORDER BY name");
        let rows: Vec<UserRow> = sqlx::query_as(&sql)
            .fetch_all(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        rows.into_iter().map(UserRow::into_domain).collect()
    }
}

/// Translates driver errors into port errors, preserving the constraint name so
/// the API can turn a violation into a field-level message.
pub(crate) fn map_sqlx(e: sqlx::Error) -> PortError {
    match &e {
        sqlx::Error::RowNotFound => PortError::NotFound,
        sqlx::Error::Database(db) => {
            let code = db.code().unwrap_or_default().to_string();
            let constraint = db.constraint().map(ToString::to_string);
            match code.as_str() {
                // unique_violation | exclusion_violation | foreign_key_violation
                "23505" | "23P01" | "23503" => PortError::Conflict { constraint },
                // check_violation -- bad input, surfaced as 400 by the API
                "23514" => PortError::CheckViolation { constraint },
                _ => PortError::Storage(e.to_string()),
            }
        }
        _ => PortError::Storage(e.to_string()),
    }
}
