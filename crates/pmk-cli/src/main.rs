//! `pmk-cli` -- operational commands.
//!
//! Migrations run here, out of process, not on server boot. The legacy server
//! executed ~1,100 lines of DDL at startup behind a hand-incremented
//! `SCHEMA_VERSION` integer, with no advisory lock, so two instances starting
//! together raced on the same DDL (Analysis 4.2).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use clap::{Parser, Subcommand};
use pmk_infra::config::Config;
use pmk_infra::db::{connect, PoolConfig};

#[derive(Parser)]
#[command(name = "pmk-cli", about = "PM Konstruct operational CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Apply pending migrations.
    Migrate {
        /// Report what would run, then roll back.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show applied and pending migrations.
    MigrateStatus,
    /// Insert a deterministic development dataset.
    Seed {
        #[arg(long, default_value = "dev")]
        profile: String,
    },
    /// Hash a password with the current algorithm (Argon2id).
    HashPassword { password: String },
    /// Print the effective configuration, with secrets redacted.
    ShowConfig,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // HashPassword needs no database or secrets, so it runs before config load.
    if let Command::HashPassword { password } = &cli.command {
        println!("{}", pmk_app::identity::password::hash(password)?);
        return Ok(());
    }

    let config = Config::load()?;

    if matches!(cli.command, Command::ShowConfig) {
        let mut redacted = config.clone();
        redacted.auth.jwt_secret = "<redacted>".into();
        redacted.storage.secret_access_key = "<redacted>".into();
        redacted.database.url = redact_url(&redacted.database.url);
        println!("{}", serde_json::to_string_pretty(&redacted)?);
        return Ok(());
    }

    let pool = connect(&PoolConfig::new(config.database.url.clone())).await?;

    match cli.command {
        Command::Migrate { dry_run } => {
            let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
                .await?;
            if dry_run {
                // `sqlx` has no dry-run, so report the gap instead of
                // pretending to apply anything.
                let applied = applied_versions(&pool).await?;
                let pending: Vec<_> = migrator
                    .iter()
                    .filter(|m| !applied.contains(&m.version))
                    .collect();
                if pending.is_empty() {
                    println!("No pending migrations.");
                } else {
                    println!("{} migration(s) would be applied:", pending.len());
                    for m in pending {
                        println!("  {} {}", m.version, m.description);
                    }
                }
                return Ok(());
            }
            // `sqlx` takes a Postgres advisory lock for the duration, so
            // concurrent runners serialise instead of racing.
            migrator.run(&pool).await?;
            println!("Migrations applied.");
        }
        Command::MigrateStatus => {
            let migrator = sqlx::migrate::Migrator::new(std::path::Path::new("./migrations"))
                .await?;
            let applied = applied_versions(&pool).await?;
            for m in migrator.iter() {
                let mark = if applied.contains(&m.version) { "applied" } else { "pending" };
                println!("  {:<8} {:<20} {}", mark, m.version, m.description);
            }
        }
        Command::Seed { profile } => {
            let n = seed(&pool, &profile).await?;
            println!("Seed profile '{profile}' applied ({n} statement groups).");
        }
        Command::HashPassword { .. } | Command::ShowConfig => unreachable!("handled above"),
    }
    Ok(())
}

async fn applied_versions(pool: &sqlx::PgPool) -> Result<Vec<i64>, sqlx::Error> {
    // Absent on a fresh database: sqlx creates it on first run.
    let rows: Result<Vec<(i64,)>, _> =
        sqlx::query_as("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await;
    Ok(rows.map(|r| r.into_iter().map(|(v,)| v).collect()).unwrap_or_default())
}

fn redact_url(url: &str) -> String {
    // postgres://user:password@host/db -> postgres://user:<redacted>@host/db
    match (url.find("://"), url.find('@')) {
        (Some(s), Some(at)) if at > s => {
            let head = &url[..s + 3];
            let creds = &url[s + 3..at];
            let tail = &url[at..];
            let user = creds.split(':').next().unwrap_or("");
            format!("{head}{user}:<redacted>{tail}")
        }
        _ => url.to_string(),
    }
}

/// Deterministic development data. Uses fixed ids so fixtures and tests can
/// reference them, and `ON CONFLICT DO NOTHING` so it is safe to re-run.
async fn seed(pool: &sqlx::PgPool, profile: &str) -> Result<usize, Box<dyn std::error::Error>> {
    if profile != "dev" && profile != "test" {
        return Err(format!("unknown seed profile '{profile}' (expected dev or test)").into());
    }

    // The legacy test accounts all share this password, and the README
    // documents it, so keeping it makes the captured fixtures reproducible.
    let hash = pmk_app::identity::password::hash("buildsmart2024")?;

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO companies (id, name, billing_onboarding_completed)
         VALUES (1, 'BuildSmart Construction', TRUE),
                (2, 'Rival Builders', TRUE)
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("SELECT setval('companies_id_seq', GREATEST((SELECT MAX(id) FROM companies), 1))")
        .execute(&mut *tx)
        .await?;

    // Company 1 covers every role plus both permission states; company 2 exists
    // solely so tenant-isolation tests have somewhere to fail to reach.
    let users: [(i32, i32, &str, &str, &str); 6] = [
        (1, 1, "James Morrison", "james.morrison@buildsmart.com.au", "MANAGER"),
        (2, 1, "Sarah Johnson", "sarah.johnson@buildsmart.com.au", "SUPERVISOR"),
        (3, 1, "Mike Chen", "mike.chen@buildsmart.com.au", "SUPERVISOR"),
        (4, 1, "Emma Davis", "emma.davis@buildsmart.com.au", "OFFICE"),
        (5, 1, "Tom Nguyen", "tom.nguyen@buildsmart.com.au", "OFFICE"),
        (6, 2, "Rival Manager", "rival.manager@rival.com.au", "MANAGER"),
    ];
    for (id, company, name, email, role) in users {
        sqlx::query(
            "INSERT INTO users (id, company_id, name, email, role, password_hash, active)
             VALUES ($1, $2, $3, $4, $5, $6, TRUE)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(company)
        .bind(name)
        .bind(email)
        .bind(role)
        .bind(&hash)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("SELECT setval('users_id_seq', GREATEST((SELECT MAX(id) FROM users), 1))")
        .execute(&mut *tx)
        .await?;

    // Mike Chen gets the `access-rules:managed` sentinel with no other grants,
    // which is the "managed-none" profile the RBAC matrix needs: everything
    // permission-gated must 403 for him (domain-rules R5).
    sqlx::query(
        "INSERT INTO user_permissions (user_id, resource, action)
         VALUES (3, 'access-rules', 'managed')
         ON CONFLICT DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO jobs (id, company_id, name, job_number, client, address, status, supervisor_id)
         VALUES (1, 1, 'Riverside Apartments Stage 2', 'BSC-2024-001', 'Riverside Developments', '1 River Rd', 'active', 2),
                (2, 1, 'Westfield Retail Fitout', 'BSC-2024-002', 'Westfield Corp', '2 Mall Way', 'active', 3),
                (3, 1, 'City Council Resurfacing', 'BSC-2024-003', 'Townsville Council', '3 Civic Pl', 'completed', 2),
                (4, 2, 'Rival Tower', 'RIV-001', 'Rival Client', '9 Other St', 'active', 6)
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("SELECT setval('jobs_id_seq', GREATEST((SELECT MAX(id) FROM jobs), 1))")
        .execute(&mut *tx)
        .await?;

    // Sarah is assigned to job 1 only, so supervisor visibility (R3) is
    // testable: she must see jobs 1 and 3 (primary) but not 2.
    sqlx::query(
        "INSERT INTO job_assignments (job_id, user_id, is_primary)
         VALUES (1, 2, TRUE), (2, 3, TRUE), (4, 6, TRUE)
         ON CONFLICT DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(5)
}
