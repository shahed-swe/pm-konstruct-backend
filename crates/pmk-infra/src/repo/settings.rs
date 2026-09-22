//! `SettingsRepository` over Postgres.
//!
//! Both tables hold one row per company, created lazily. Reads therefore fall
//! back to the domain defaults rather than failing, so a company that has
//! never opened the settings page looks the same as one that has and changed
//! nothing.
//!
//! Every scoped call sets the tenant GUC first. `company_branding` and
//! `email_settings` are RLS-protected on `current_company_id()`, and binding
//! the company id into the predicate is *not* a substitute: without the GUC
//! the policy denies the row, so reads come back empty and writes fail. The
//! one call that genuinely has no tenant -- the login page's branding, read
//! before anyone has authenticated -- goes through the SECURITY DEFINER
//! function from migration 0011 instead.

use async_trait::async_trait;
use pmk_domain::settings::{
    default_branding, Branding, BrandingInput, EmailSendMode, EmailSettings, JobDisplayMode,
};
use pmk_domain::tenant::TenantScope;
use pmk_ports::repository::SettingsRepository;
use pmk_ports::PortResult;
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::db::tenant::set_tenant;

use super::user::map_sqlx;

fn branding_row(r: &sqlx::postgres::PgRow) -> Branding {
    let d = default_branding();
    Branding {
        company_name: r.get("company_name"),
        logo_url: r.get("logo_url"),
        banner_url: r.get("banner_url"),
        primary_color: r.get("primary_color"),
        sidebar_color: r.get("sidebar_color"),
        // An unrecognised stored value falls back to the default rather than
        // failing the page: the column is free text and predates the check.
        job_display_mode: JobDisplayMode::parse(r.get("job_display_mode"))
            .unwrap_or(d.job_display_mode),
        email_send_mode: EmailSendMode::parse(r.get("email_send_mode"))
            .unwrap_or(d.email_send_mode),
        updated_at: r.get("updated_at"),
    }
}

const BRANDING_COLS: &str = "company_name, logo_url, banner_url, primary_color, sidebar_color, \
                             job_display_mode, email_send_mode, updated_at";

#[derive(Debug, Clone)]
pub struct PgSettingsRepository {
    pool: PgPool,
}

impl PgSettingsRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self, scope: TenantScope) -> PortResult<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        set_tenant(&mut tx, scope).await.map_err(map_sqlx)?;
        Ok(tx)
    }
}

#[async_trait]
impl SettingsRepository for PgSettingsRepository {
    async fn branding(&self, scope: TenantScope) -> PortResult<Branding> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(&format!("SELECT {BRANDING_COLS} FROM company_branding"))
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.as_ref().map_or_else(default_branding, branding_row))
    }

    async fn default_branding(&self) -> PortResult<Branding> {
        // No session exists yet, so there is no tenant to set. The function
        // exposes one row of one table and nothing else (migration 0011).
        let r = sqlx::query(&format!(
            "SELECT {BRANDING_COLS} FROM public_default_branding()"
        ))
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(r.as_ref().map_or_else(default_branding, branding_row))
    }

    async fn set_branding(
        &self,
        scope: TenantScope,
        input: &BrandingInput,
    ) -> PortResult<Branding> {
        let mut tx = self.begin(scope).await?;
        // The images are not in the SET list: they are replaced by their own
        // endpoints, and including them here would clear a logo every time
        // someone changed a colour.
        let r = sqlx::query(&format!(
            "INSERT INTO company_branding \
               (company_id, company_name, primary_color, sidebar_color, \
                job_display_mode, email_send_mode) \
             VALUES ($1,$2,$3,$4,$5,$6) \
             ON CONFLICT (company_id) DO UPDATE \
               SET company_name = EXCLUDED.company_name, \
                   primary_color = EXCLUDED.primary_color, \
                   sidebar_color = EXCLUDED.sidebar_color, \
                   job_display_mode = EXCLUDED.job_display_mode, \
                   email_send_mode = EXCLUDED.email_send_mode, \
                   updated_at = NOW() \
             RETURNING {BRANDING_COLS}"
        ))
        .bind(scope.company_id().get())
        .bind(input.company_name.trim())
        .bind(&input.primary_color)
        .bind(&input.sidebar_color)
        .bind(input.job_display_mode.as_str())
        .bind(input.email_send_mode.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(branding_row(&r))
    }

    async fn set_branding_image(
        &self,
        scope: TenantScope,
        banner: bool,
        object_key: Option<&str>,
    ) -> PortResult<Option<String>> {
        let mut tx = self.begin(scope).await?;
        let column = if banner { "banner_url" } else { "logo_url" };
        // RETURNING the *old* value: the caller sweeps the object it replaced,
        // and reading it separately would race with a concurrent change.
        let sql = format!(
            "INSERT INTO company_branding (company_id, {column}) VALUES ($1, $2) \
             ON CONFLICT (company_id) DO UPDATE \
               SET {column} = EXCLUDED.{column}, updated_at = NOW() \
             RETURNING (SELECT b.{column} FROM company_branding b WHERE b.company_id = $1) \
               AS previous"
        );
        let r = sqlx::query(&sql)
            .bind(scope.company_id().get())
            .bind(object_key)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.get("previous"))
    }

    async fn email_settings(&self, scope: TenantScope) -> PortResult<EmailSettings> {
        let mut tx = self.begin(scope).await?;
        let r = sqlx::query(
            "SELECT smtp_host, smtp_port, smtp_user, smtp_from, smtp_secure \
             FROM email_settings",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        Ok(r.map_or_else(EmailSettings::default, |r| EmailSettings {
            smtp_host: r.get("smtp_host"),
            smtp_port: r.get("smtp_port"),
            smtp_user: r.get("smtp_user"),
            smtp_from: r.get("smtp_from"),
            smtp_secure: r.get::<Option<bool>, _>("smtp_secure").unwrap_or(false),
            // Never read into the struct the API serialises.
            smtp_pass: None,
        }))
    }

    async fn has_smtp_password(&self, scope: TenantScope) -> PortResult<bool> {
        let mut tx = self.begin(scope).await?;
        let r: Option<bool> = sqlx::query_scalar(
            "SELECT smtp_pass IS NOT NULL AND smtp_pass <> '' FROM email_settings",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx)?
        .flatten();
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.unwrap_or(false))
    }

    async fn smtp_password(&self, scope: TenantScope) -> PortResult<Option<String>> {
        let mut tx = self.begin(scope).await?;
        let r: Option<Option<String>> = sqlx::query_scalar("SELECT smtp_pass FROM email_settings")
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(r.flatten())
    }

    async fn set_email_settings(
        &self,
        scope: TenantScope,
        input: &EmailSettings,
    ) -> PortResult<EmailSettings> {
        let mut tx = self.begin(scope).await?;
        // COALESCE on the password: a `None` means the form was submitted
        // without retyping it, which must not blank the stored one.
        sqlx::query(
            "INSERT INTO email_settings \
               (company_id, smtp_host, smtp_port, smtp_user, smtp_pass, smtp_from, smtp_secure) \
             VALUES ($1,$2,$3,$4,$5,$6,$7) \
             ON CONFLICT (company_id) DO UPDATE \
               SET smtp_host = EXCLUDED.smtp_host, \
                   smtp_port = EXCLUDED.smtp_port, \
                   smtp_user = EXCLUDED.smtp_user, \
                   smtp_pass = COALESCE(EXCLUDED.smtp_pass, email_settings.smtp_pass), \
                   smtp_from = EXCLUDED.smtp_from, \
                   smtp_secure = EXCLUDED.smtp_secure, \
                   updated_at = NOW()",
        )
        .bind(scope.company_id().get())
        .bind(input.smtp_host.as_deref())
        .bind(input.smtp_port)
        .bind(input.smtp_user.as_deref())
        .bind(input.smtp_pass.as_deref())
        .bind(input.smtp_from.as_deref())
        .bind(input.smtp_secure)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;

        self.email_settings(scope).await
    }
}
