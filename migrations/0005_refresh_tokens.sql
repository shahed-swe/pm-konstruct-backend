-- PM Konstruct v2 -- refresh tokens (Phase 5)
--
-- The legacy system issued a single 8h JWT with no refresh and no revocation:
-- logout was client-side only, and a stolen token stayed valid for its full
-- life. This replaces it with a short access token plus a rotating refresh
-- token that can actually be revoked.
--
-- Tokens are stored as SHA-256 hashes, never in plaintext: a database read
-- must not yield usable credentials.
--
-- `family_id` groups every token descended from one login. Presenting an
-- already-used token means it leaked, so the whole family is revoked -- the
-- standard reuse-detection pattern.

CREATE TABLE refresh_tokens (
  id          BIGSERIAL PRIMARY KEY,
  user_id     INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
  token_hash  TEXT NOT NULL UNIQUE,
  family_id   UUID NOT NULL,
  issued_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at  TIMESTAMPTZ NOT NULL,
  used_at     TIMESTAMPTZ,
  revoked_at  TIMESTAMPTZ,
  user_agent  TEXT,
  ip          INET,
  CONSTRAINT refresh_tokens_expiry_after_issue CHECK (expires_at > issued_at)
);

CREATE INDEX idx_refresh_tokens_user   ON refresh_tokens (user_id);
CREATE INDEX idx_refresh_tokens_family ON refresh_tokens (family_id);
-- Drives the retention sweep in the worker.
CREATE INDEX idx_refresh_tokens_expiry ON refresh_tokens (expires_at)
  WHERE revoked_at IS NULL;

-- Refresh tokens are looked up by hash *before* any tenant is known, so this
-- table cannot be tenant-scoped the way the rest are. It is reachable only by
-- the token hash, which is unguessable, and rows carry no tenant data beyond
-- user_id. pmk_app therefore gets direct grants and no RLS policy.
GRANT SELECT, INSERT, UPDATE, DELETE ON refresh_tokens TO pmk_app;
GRANT USAGE, SELECT ON SEQUENCE refresh_tokens_id_seq TO pmk_app;
