use std::collections::BTreeMap;

use serde::de::DeserializeOwned;

use crate::error::{CliError, CliResult};

/// A minimal, dependency-free argument model.
///
/// The grammar is deliberately small: leading positional tokens (the command
/// group and subcommand) followed by `--flag value` pairs. A flag with no
/// following value (or followed by another `--flag`) is treated as a boolean
/// switch. Flags may repeat; every value is preserved in order.
#[derive(Debug, Default, Clone)]
pub struct ParsedArgs {
    pub positionals: Vec<String>,
    flags: BTreeMap<String, Vec<String>>,
}

impl ParsedArgs {
    /// Parse an iterator of raw tokens (the program name must already be
    /// stripped by the caller).
    pub fn parse<I: Iterator<Item = String>>(tokens: I) -> CliResult<Self> {
        let mut parsed = ParsedArgs::default();
        let mut tokens = tokens.peekable();
        while let Some(token) = tokens.next() {
            if let Some(key) = token.strip_prefix("--") {
                if key.is_empty() {
                    return Err(CliError::Usage(
                        "encountered an empty '--' flag".to_string(),
                    ));
                }
                // A value follows unless the next token is another flag or the
                // stream is exhausted, in which case this is a boolean switch.
                let takes_value = matches!(tokens.peek(), Some(next) if !next.starts_with("--"));
                let value = if takes_value {
                    tokens.next().unwrap_or_default()
                } else {
                    "true".to_string()
                };
                parsed.flags.entry(key.to_string()).or_default().push(value);
            } else {
                parsed.positionals.push(token);
            }
        }
        Ok(parsed)
    }

    /// The positional at `index`, if present.
    pub fn positional(&self, index: usize) -> Option<&str> {
        self.positionals.get(index).map(String::as_str)
    }

    /// Whether a boolean switch was set.
    pub fn has_flag(&self, key: &str) -> bool {
        self.flags.contains_key(key)
    }

    /// The last value provided for a flag, if any.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.flags
            .get(key)
            .and_then(|values| values.last())
            .map(String::as_str)
    }

    /// The last value provided for a flag, or an error if it is missing.
    pub fn require(&self, key: &str) -> CliResult<&str> {
        self.get(key)
            .ok_or_else(|| CliError::MissingFlag(key.to_string()))
    }

    /// The last value for a flag, or a default.
    pub fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).unwrap_or(default)
    }

    /// All values provided for a repeatable flag, in order.
    pub fn all(&self, key: &str) -> Vec<String> {
        self.flags.get(key).cloned().unwrap_or_default()
    }

    /// All comma-separated values across every occurrence of a flag, trimmed
    /// and with empties dropped. `--actions read,write --actions execute`
    /// yields `[read, write, execute]`.
    pub fn csv(&self, key: &str) -> Vec<String> {
        self.all(key)
            .iter()
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()
    }
}

/// Parse a snake_case token into a core enum by reusing its serde
/// representation, so the CLI never duplicates the enum-to-string mapping.
pub fn parse_enum<T: DeserializeOwned>(field: &str, value: &str) -> CliResult<T> {
    // Core enums serialize as plain snake_case strings with no characters that
    // require JSON escaping, so quoting is sufficient and safe.
    serde_json::from_str::<T>(&format!("\"{value}\"")).map_err(|_| CliError::invalid(field, value))
}

/// Parse a required flag whose value maps to a core enum.
pub fn require_enum<T: DeserializeOwned>(args: &ParsedArgs, field: &str) -> CliResult<T> {
    parse_enum(field, args.require(field)?)
}

/// Parse an optional unsigned integer flag with a default.
pub fn get_u64_or(args: &ParsedArgs, field: &str, default: u64) -> CliResult<u64> {
    match args.get(field) {
        Some(raw) => raw
            .parse::<u64>()
            .map_err(|_| CliError::invalid(field, raw)),
        None => Ok(default),
    }
}
