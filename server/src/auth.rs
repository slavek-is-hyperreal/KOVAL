use bcrypt::{hash, verify, BcryptError};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use schema::Token;
use crate::db;

#[derive(Debug, PartialEq, Clone)]
pub enum AuthError {
    InvalidToken,
    InactiveToken,
    RateLimitExceeded,
    DatabaseError(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::InvalidToken => write!(f, "Invalid token"),
            AuthError::InactiveToken => write!(f, "Inactive token"),
            AuthError::RateLimitExceeded => write!(f, "Rate limit exceeded"),
            AuthError::DatabaseError(err) => write!(f, "Database error: {}", err),
        }
    }
}

impl std::error::Error for AuthError {}

impl From<rusqlite::Error> for AuthError {
    fn from(err: rusqlite::Error) -> Self {
        AuthError::DatabaseError(err.to_string())
    }
}

impl From<BcryptError> for AuthError {
    fn from(_: BcryptError) -> Self {
        AuthError::InvalidToken
    }
}

fn get_bcrypt_cost() -> u32 {
    let default_cost = if cfg!(test) { 4 } else { 12 };
    match std::env::var("KOVAL_BCRYPT_COST") {
        Ok(val) => match val.parse::<u32>() {
            Ok(cost) if (4..=31).contains(&cost) => cost,
            _ => {
                eprintln!("Warning: Invalid KOVAL_BCRYPT_COST '{}'. Must be between 4 and 31. Falling back to default: {}.", val, default_cost);
                default_cost
            }
        },
        Err(_) => default_cost,
    }
}

/// Hashes a plain text token using bcrypt
pub fn hash_token(raw_token: &str) -> Result<String, BcryptError> {
    hash(raw_token, get_bcrypt_cost())
}

/// Authenticates a token and applies sliding window rate-limiting
pub fn authenticate_and_rate_limit(
    conn: &Connection,
    raw_token: &str,
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Token, AuthError> {
    // 1. Fetch active tokens
    let active_tokens = db::get_active_tokens(conn)?;

    // 2. Find matching token using bcrypt
    let mut matched_token: Option<Token> = None;
    for t in active_tokens {
        if verify(raw_token, &t.token_hash).unwrap_or(false) {
            matched_token = Some(t);
            break;
        }
    }

    let token = match matched_token {
        Some(t) => t,
        None => return Err(AuthError::InvalidToken),
    };

    if !token.is_active {
        return Err(AuthError::InactiveToken);
    }

    // 3. Sliding window check: last 60 seconds
    let window_duration = chrono::Duration::seconds(60);
    let since_time = now - window_duration;
    let since_str = since_time.to_rfc3339();

    // Prune old rate limit entries to keep table clean
    db::prune_rate_limits(conn, &since_str)?;

    // Calculate current sliding count
    let current_requests = db::get_sliding_window_count(conn, token.id, &since_str)?;

    if current_requests >= limit {
        return Err(AuthError::RateLimitExceeded);
    }

    // Increment count for current window (store with millisecond precision)
    let now_str = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    db::increment_rate_limit(conn, token.id, &now_str)?;

    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;

    fn setup_test_db() -> Connection {
        init_db(":memory:").unwrap()
    }

    #[test]
    fn test_auth_verify_and_rate_limit() {
        let conn = setup_test_db();
        let raw_token = "koval_test_token_123";
        let hashed = hash_token(raw_token).unwrap();
        let created_at = Utc::now().to_rfc3339();

        let token_id = db::insert_token(&conn, &hashed, "Dev Token", &created_at).unwrap();
        assert!(token_id > 0);

        let now = Utc::now();

        // 1. Valid token verifies
        let auth_res = authenticate_and_rate_limit(&conn, raw_token, now, 3);
        assert!(auth_res.is_ok());
        let token = auth_res.unwrap();
        assert_eq!(token.id, token_id);
        assert_eq!(token.name, "Dev Token");

        // 2. Invalid token rejects
        let invalid_res = authenticate_and_rate_limit(&conn, "wrong_token", now, 3);
        assert_eq!(invalid_res, Err(AuthError::InvalidToken));

        // 3. Rate limit triggers at threshold
        // We already made 1 request. Let's make 2 more (total 3, limit is 3)
        let res2 = authenticate_and_rate_limit(&conn, raw_token, now + chrono::Duration::milliseconds(10), 3);
        assert!(res2.is_ok());
        let res3 = authenticate_and_rate_limit(&conn, raw_token, now + chrono::Duration::milliseconds(20), 3);
        assert!(res3.is_ok());

        // 4th request should exceed rate limit
        let res4 = authenticate_and_rate_limit(&conn, raw_token, now + chrono::Duration::milliseconds(30), 3);
        assert_eq!(res4, Err(AuthError::RateLimitExceeded));

        // 4. Rate limit resets after window (60 seconds later)
        let later = now + chrono::Duration::seconds(61);
        let res_reset = authenticate_and_rate_limit(&conn, raw_token, later, 3);
        assert!(res_reset.is_ok());
    }

    #[test]
    fn test_bcrypt_cost_env_var() {
        std::env::remove_var("KOVAL_BCRYPT_COST");
        let h1 = hash_token("test").unwrap();
        let cost1 = h1.split('$').nth(2).unwrap().parse::<u32>().unwrap();
        assert_eq!(cost1, 4);

        std::env::set_var("KOVAL_BCRYPT_COST", "5");
        let h2 = hash_token("test").unwrap();
        let cost2 = h2.split('$').nth(2).unwrap().parse::<u32>().unwrap();
        assert_eq!(cost2, 5);

        std::env::set_var("KOVAL_BCRYPT_COST", "3");
        let h3 = hash_token("test").unwrap();
        let cost3 = h3.split('$').nth(2).unwrap().parse::<u32>().unwrap();
        assert_eq!(cost3, 4);

        std::env::set_var("KOVAL_BCRYPT_COST", "abc");
        let h4 = hash_token("test").unwrap();
        let cost4 = h4.split('$').nth(2).unwrap().parse::<u32>().unwrap();
        assert_eq!(cost4, 4);

        std::env::remove_var("KOVAL_BCRYPT_COST");
    }
}
