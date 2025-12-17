//! Configuration validation utilities.

use std::time::Duration;

use thiserror::Error;

/// Configuration error types.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to read configuration file.
    #[error("failed to read config file: {0}")]
    IoError(#[from] std::io::Error),

    /// Failed to parse YAML configuration.
    #[error("failed to parse YAML config: {0}")]
    ParseError(#[from] serde_yaml::Error),

    /// Configuration validation failed.
    #[error("config validation error: {0}")]
    ValidationError(String),
}

/// Parse duration string using humantime.
///
/// Supports various formats: `30s`, `1m`, `5m30s`, `1h`, `2h30m`, `1d`, `100ms`, etc.
///
/// # Examples
///
/// ```
/// use oculus::config::parse_duration;
///
/// assert_eq!(parse_duration("30s").unwrap().as_secs(), 30);
/// assert_eq!(parse_duration("1m").unwrap().as_secs(), 60);
/// assert_eq!(parse_duration("2h").unwrap().as_secs(), 7200);
/// assert_eq!(parse_duration("1h30m").unwrap().as_secs(), 5400);
/// ```
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("duration string is empty".to_string());
    }
    humantime::parse_duration(s).map_err(|e| e.to_string())
}

/// Expand environment variables in a string.
/// Supports ${VAR} and ${VAR:-default} syntax.
pub fn expand_env_vars(input: &str) -> String {
    static ENV_VAR_REGEX: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

    let regex = ENV_VAR_REGEX.get_or_init(|| {
        regex::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}")
            .expect("failed to compile env var regex")
    });

    regex
        .replace_all(input, |caps: &regex::Captures| {
            let var_name = &caps[1];
            let default_value = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            std::env::var(var_name).unwrap_or_else(|_| default_value.to_string())
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_valid() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("1m").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("30x").is_err());
        assert!(parse_duration("30").is_err());
    }

    #[test]
    fn test_parse_duration_extended_formats() {
        assert_eq!(parse_duration("100ms").unwrap(), Duration::from_millis(100));
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
        assert_eq!(parse_duration("2h 30m").unwrap(), Duration::from_secs(9000));
    }

    #[test]
    fn test_expand_env_vars_no_vars() {
        assert_eq!(expand_env_vars("hello world"), "hello world");
    }

    #[test]
    fn test_expand_env_vars_with_default() {
        // Use a variable that definitely doesn't exist
        let result = expand_env_vars("Bearer ${NONEXISTENT_TOKEN_12345:-default_token}");
        assert_eq!(result, "Bearer default_token");
    }

    #[test]
    fn test_expand_env_vars_from_env() {
        // SAFETY: This test runs in isolation and only modifies a test-specific variable.
        unsafe {
            std::env::set_var("TEST_VAR_EXPAND", "secret_value");
        }
        let result = expand_env_vars("Authorization: ${TEST_VAR_EXPAND}");
        assert_eq!(result, "Authorization: secret_value");
        // SAFETY: Cleanup test variable.
        unsafe {
            std::env::remove_var("TEST_VAR_EXPAND");
        }
    }
}
