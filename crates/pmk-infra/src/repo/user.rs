//! `UserRepository` over Postgres.

use async_trait::async_trait;
use pmk_domain::access::{Permission, Role};
use pmk_domain::identity::accounts::UserInput;
use pmk_domain::ids::{JobId, UserId};
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

    async fn email_recipients_for_job(
        &self,
        scope: TenantScope,
        job: JobId,
    ) -> PortResult<Vec<String>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        // Inactive accounts are excluded: someone who has left the company
        // must not keep receiving its site diaries.
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT u.email FROM users u \
             WHERE u.active \
               AND ( u.role IN ('MANAGER','OFFICE') \
                     OR u.id = (SELECT j.supervisor_id FROM jobs j WHERE j.id = $1) \
                     OR EXISTS (SELECT 1 FROM job_assignments a \
                                WHERE a.job_id = $1 AND a.user_id = u.id) ) \
             ORDER BY u.email",
        )
        .bind(job.get())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(rows)
    }

    async fn find(&self, scope: TenantScope, id: UserId) -> PortResult<Option<User>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        let sql = format!("SELECT {USER_COLUMNS} FROM users WHERE id = $1");
        let row: Option<UserRow> = sqlx::query_as(&sql)
            .bind(id.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        row.map(UserRow::into_domain).transpose()
    }

    async fn active_count(&self, scope: TenantScope) -> PortResult<i64> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE active")
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(n)
    }

    async fn create(
        &self,
        scope: TenantScope,
        input: &UserInput,
        password_hash: &str,
    ) -> PortResult<User> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        // `company_id` is written explicitly rather than defaulted: the RLS
        // policy checks it, and a mismatch should fail loudly here rather than
        // create a row nobody can see.
        let sql = format!(
            "INSERT INTO users (company_id, name, email, role, phone, active, password_hash) \
             VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING {USER_COLUMNS}"
        );
        let row: UserRow = sqlx::query_as(&sql)
            .bind(scope.company_id().get())
            .bind(input.name.trim())
            .bind(input.normalised_email())
            .bind(input.role.as_str())
            .bind(input.phone.as_deref())
            .bind(input.active)
            .bind(password_hash)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        row.into_domain()
    }

    async fn update(
        &self,
        scope: TenantScope,
        id: UserId,
        input: &UserInput,
        password_hash: Option<&str>,
    ) -> PortResult<Option<User>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        // `password_changed_at` moves only when the hash does, so it stays an
        // accurate record of when the credential last changed.
        let sql = format!(
            "UPDATE users SET name = $2, email = $3, role = $4, phone = $5, active = $6, \
                    password_hash = COALESCE($7, password_hash), \
                    password_changed_at = CASE WHEN $7::text IS NULL \
                        THEN password_changed_at ELSE NOW() END \
             WHERE id = $1 RETURNING {USER_COLUMNS}"
        );
        let row: Option<UserRow> = sqlx::query_as(&sql)
            .bind(id.get())
            .bind(input.name.trim())
            .bind(input.normalised_email())
            .bind(input.role.as_str())
            .bind(input.phone.as_deref())
            .bind(input.active)
            .bind(password_hash)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        row.map(UserRow::into_domain).transpose()
    }

    async fn delete(&self, scope: TenantScope, id: UserId) -> PortResult<bool> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        let n = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .rows_affected();
        tx.commit().await.map_err(map_sqlx)?;
        Ok(n > 0)
    }

    async fn set_permissions(
        &self,
        scope: TenantScope,
        id: UserId,
        permissions: &[Permission],
    ) -> PortResult<()> {
        let resources: Vec<String> = permissions.iter().map(|p| p.resource.clone()).collect();
        let actions: Vec<String> = permissions.iter().map(|p| p.action.clone()).collect();

        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        // Replace, not merge, and in one transaction: a manager who removes a
        // grant must not see it survive because the insert half failed.
        sqlx::query("DELETE FROM user_permissions WHERE user_id = $1")
            .bind(id.get())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        if !permissions.is_empty() {
            sqlx::query(
                "INSERT INTO user_permissions (user_id, resource, action) \
                 SELECT $1, * FROM unnest($2::text[], $3::text[]) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(id.get())
            .bind(&resources)
            .bind(&actions)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(())
    }

    async fn issue_recovery_code(
        &self,
        scope: TenantScope,
        id: UserId,
        token_hash: &str,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> PortResult<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        // Any outstanding code is spent first, so issuing a new one cannot
        // leave two valid codes in circulation.
        sqlx::query(
            "UPDATE password_reset_tokens SET used_at = NOW() \
             WHERE user_id = $1 AND used_at IS NULL",
        )
        .bind(id.get())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;

        sqlx::query(
            "INSERT INTO password_reset_tokens (user_id, token, expires_at) VALUES ($1,$2,$3)",
        )
        .bind(id.get())
        .bind(token_hash)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(())
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
