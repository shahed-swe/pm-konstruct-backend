//! `pmk-cli` -- operational commands.
//!
//! Migrations run here, out of process, not on server boot. The legacy server
//! executed ~1,100 lines of DDL at startup behind a hand-incremented
//! `SCHEMA_VERSION` integer, with no advisory lock, so two instances starting
//! together raced on the same DDL (Analysis 4.2).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

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
            let migrator =
                sqlx::migrate::Migrator::new(std::path::Path::new("./migrations")).await?;
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
            let migrator =
                sqlx::migrate::Migrator::new(std::path::Path::new("./migrations")).await?;
            let applied = applied_versions(&pool).await?;
            for m in migrator.iter() {
                let mark = if applied.contains(&m.version) {
                    "applied"
                } else {
                    "pending"
                };
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
    Ok(rows
        .map(|r| r.into_iter().map(|(v,)| v).collect())
        .unwrap_or_default())
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
        (
            1,
            1,
            "James Morrison",
            "james.morrison@buildsmart.com.au",
            "MANAGER",
        ),
        (
            2,
            1,
            "Sarah Johnson",
            "sarah.johnson@buildsmart.com.au",
            "SUPERVISOR",
        ),
        (
            3,
            1,
            "Mike Chen",
            "mike.chen@buildsmart.com.au",
            "SUPERVISOR",
        ),
        (4, 1, "Emma Davis", "emma.davis@buildsmart.com.au", "OFFICE"),
        (5, 1, "Tom Nguyen", "tom.nguyen@buildsmart.com.au", "OFFICE"),
        (
            6,
            2,
            "Rival Manager",
            "rival.manager@rival.com.au",
            "MANAGER",
        ),
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

    // Tom Nguyen is the other half of R5: the sentinel *plus* grants, so his
    // stored permissions actually take effect. He is OFFICE with
    // site-diary:write, which no other fixture provides -- without him the
    // rule that office staff may delete only their own uploads is unreachable
    // over HTTP, because every other office user is stopped by the permission
    // gate first.
    sqlx::query(
        "INSERT INTO user_permissions (user_id, resource, action)
         VALUES (5, 'access-rules', 'managed'),
                (5, 'site-diary', 'read'),
                (5, 'site-diary', 'write'),
                (5, 'jobs', 'read')
         ON CONFLICT DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        // Dates are relative to CURRENT_DATE so the calendar always has
        // something in the current month. Job 2 is deliberately open-ended
        // (no end_date): the calendar must treat that as running forever
        // rather than dropping it, and job 3 is deliberately in the past.
        "INSERT INTO jobs
           (id, company_id, name, job_number, client, address, status, supervisor_id,
            start_date, end_date)
         VALUES (1, 1, 'Riverside Apartments Stage 2', 'BSC-2024-001', 'Riverside Developments', '1 River Rd', 'active', 2,
                 CURRENT_DATE - 60, CURRENT_DATE + 60),
                (2, 1, 'Westfield Retail Fitout', 'BSC-2024-002', 'Westfield Corp', '2 Mall Way', 'active', 3,
                 CURRENT_DATE - 30, NULL),
                (3, 1, 'City Council Resurfacing', 'BSC-2024-003', 'Townsville Council', '3 Civic Pl', 'completed', 2,
                 CURRENT_DATE - 400, CURRENT_DATE - 300),
                (4, 2, 'Rival Tower', 'RIV-001', 'Rival Client', '9 Other St', 'active', 6,
                 CURRENT_DATE - 10, CURRENT_DATE + 10)
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

    // Call-forward items with dates fixed relative to CURRENT_DATE, so the
    // overdue and delay-severity panels have one item in each of their four
    // buckets no matter when the seed runs. Ids are pinned above 100 to stay
    // clear of anything a test creates.
    //
    // The last three exist to be *excluded*: a HEADER is a grouping label
    // rather than work, and completed or on-hold items are not overdue however
    // far past their date they are.
    sqlx::query(
        "INSERT INTO call_forward
           (id, job_id, title, item_type, est_start, est_finish, actual_finish, status, sort_order)
         VALUES
           (101, 1, 'Frame inspection',   'TASK',        CURRENT_DATE - 10, CURRENT_DATE - 3,  NULL, 'in_progress', 1),
           (102, 1, 'Roof battens',       'TASK',        CURRENT_DATE - 20, CURRENT_DATE - 10, NULL, 'not_started', 2),
           (103, 1, 'Window delivery',    'TASK',        CURRENT_DATE - 30, CURRENT_DATE - 20, NULL, 'in_progress', 3),
           (104, 1, 'Slab pour',          'TASK',        CURRENT_DATE - 60, CURRENT_DATE - 40, NULL, 'not_started', 4),
           (105, 1, 'Lock-up claim',      'STAGE_CLAIM', CURRENT_DATE - 2,  CURRENT_DATE + 5,  NULL, 'in_progress', 5),
           (106, 1, 'Fit-off stage',      'HEADER',      NULL,              CURRENT_DATE - 50, NULL, 'not_started', 6),
           (107, 1, 'Plumbing rough-in',  'TASK',        CURRENT_DATE - 40, CURRENT_DATE - 35, CURRENT_DATE - 34, 'completed', 7),
           (108, 1, 'Tiling',             'TASK',        CURRENT_DATE - 40, CURRENT_DATE - 35, NULL, 'on_hold', 8),
           (109, 2, 'Shopfront glazing',  'TASK',        CURRENT_DATE - 15, CURRENT_DATE - 9,  NULL, 'not_started', 1),
           (110, 4, 'Rival slab',         'TASK',        CURRENT_DATE - 15, CURRENT_DATE - 9,  NULL, 'not_started', 1)
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "SELECT setval('call_forward_id_seq', GREATEST((SELECT MAX(id) FROM call_forward), 1))",
    )
    .execute(&mut *tx)
    .await?;

    // Diary entries with weather, so the weather-impact and inspection
    // reports have something real to classify. Ids are pinned above 200.
    //
    // 201 is a heavy-rainfall day, 202 an adverse condition with no
    // measurement, 203 a fine day whose issues blame the weather, 204 a
    // plainly fine day, and 205 light rain -- rainy, but not enough to stop
    // work, which is the case that separates the rainy-day count from the
    // impact count.
    sqlx::query(
        "INSERT INTO site_diary
           (id, job_id, author_id, date, time, work_completed, weather_condition,
            temperature, rainfall_mm, wind_speed_kmh, issues, safety_notes, workforce)
         VALUES
           (201, 1, 2, CURRENT_DATE - 5, '07:30', 'Rained off',        'Heavy Rain', 14.5, 32.0, 25.0, NULL, 'Toolbox talk held', 4),
           (202, 1, 2, CURRENT_DATE - 4, '07:30', 'Storm delays',      'Thunderstorm', 18.0, 0.0, 45.0, NULL, NULL, 6),
           (203, 1, 2, CURRENT_DATE - 3, '07:30', 'Access blocked',    'Sunny', 26.0, 0.0, 10.0, 'Flooding across the access road', NULL, 8),
           (204, 1, 2, CURRENT_DATE - 2, '07:30', 'Framing continued', 'Sunny', 24.0, 0.0, 12.0, NULL, 'Harness check', 10),
           (205, 1, 2, CURRENT_DATE - 1, '07:30', 'Slow going',        'Light rain', 19.5, 2.0, 15.0, NULL, NULL, 7)
         ON CONFLICT (id) DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "SELECT setval('site_diary_id_seq', GREATEST((SELECT MAX(id) FROM site_diary), 1))",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(7)
}
