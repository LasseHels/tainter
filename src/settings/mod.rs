use std::borrow::Cow;
use std::str::FromStr;

use config::{Config, ConfigError};
use regex::Regex;
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use strum_macros::EnumString;
use thiserror::Error;
use validator::{Validate, ValidationError, ValidationErrors};

#[derive(Deserialize, Debug)]
struct Server {
    host: String,
    port: u16,
}

// https://serde.rs/field-attrs.html#deserialize_with.
// https://stackoverflow.com/a/46755370.
fn tracing_level_from_string<'de, D>(deserializer: D) -> Result<tracing::Level, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;

    tracing::Level::from_str(s.as_str()).map_err(Error::custom)
}

#[derive(Deserialize, Debug)]
struct Log {
    #[serde(deserialize_with = "tracing_level_from_string")]
    max_level: tracing::Level,
}

#[derive(Deserialize, Validate, Debug)]
pub struct Settings {
    server: Server,
    log: Log,
    #[validate(nested)]
    reconciler: Reconciler,
}

#[derive(Deserialize, Validate, Debug)]
struct Reconciler {
    #[validate(nested)]
    matchers: Vec<Matcher>,
}

#[derive(Deserialize, Validate, Debug)]
struct Matcher {
    #[validate(nested)]
    taint: Taint,
    #[validate(nested)]
    conditions: Vec<Condition>,
}

#[derive(Debug, PartialEq, Deserialize, EnumString)]
enum TaintEffect {
    NoSchedule,
    PreferNoSchedule,
    NoExecute,
}

#[derive(Deserialize, Validate, Debug)]
struct Taint {
    effect: TaintEffect,
    #[validate(length(min = 1))]
    key: String,
    #[validate(length(min = 1))]
    value: String,
}

#[derive(Deserialize, Validate, Debug)]
struct Condition {
    #[serde(rename = "type")]
    #[validate(custom(function = "validate_regex"))]
    type_: String,
    #[validate(custom(function = "validate_regex"))]
    status: String,
}

fn validate_regex(value: &str) -> Result<(), ValidationError> {
    let res = Regex::new(value);

    if res.is_err() {
        let msg = format!("{} ", res.err().unwrap());
        return Err(ValidationError {
            code: Default::default(),
            message: Some(Cow::from(msg)),
            params: Default::default(),
        });
    }

    Ok(())
}

#[derive(Error, Debug)]
pub enum NewSettingsError {
    #[error("error reading settings file {0}")]
    ReadFile(#[from] ConfigError),
    #[error("error validating settings {0}")]
    Validate(#[from] ValidationErrors),
}

impl Settings {
    pub fn new(path: &str) -> Result<Self, NewSettingsError> {
        let config = Config::builder()
            .add_source(config::File::with_name(path))
            .build()?;

        let settings = config.try_deserialize::<Settings>()?;

        settings.validate()?;

        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use crate::settings::Settings;
    use crate::settings::TaintEffect;

    // https://github.com/frondeus/test-case/wiki.
    #[test_case("invalid path", "error reading settings file configuration file \"invalid path\" not found" ; "returns error on invalid path")]
    #[test_case("src/settings/testfiles/invalid.toml", "error reading settings file TOML parse error at line 1, column 8\n  |\n1 | invalid\n  |        ^\nexpected `.`, `=`\n in src/settings/testfiles/invalid.toml" ; "returns error on invalid configuration file content")]
    #[test_case("src/settings/testfiles/incomplete.toml", "error reading settings file missing field `server`" ; "returns error on incomplete configuration")]
    #[test_case("src/settings/testfiles/invalid_log_max_level.toml", "error reading settings file error parsing level: expected one of \"error\", \"warn\", \"info\", \"debug\", \"trace\", or a number 1-5" ; "returns error on invalid log max_level")]
    #[test_case("src/settings/testfiles/invalid_taint_effect.toml", "error reading settings file enum TaintEffect does not have variant constructor Nope" ; "returns error on invalid taint effect")]
    #[test_case("src/settings/testfiles/empty_taint_value.toml", "error validating settings reconciler.matchers[0].taint.value: Validation error: length" ; "returns error on empty taint value")]
    #[test_case("src/settings/testfiles/empty_taint_key.toml", "error validating settings reconciler.matchers[0].taint.key: Validation error: length" ; "returns error on empty taint key")]
    #[test_case("src/settings/testfiles/invalid_condition_type_regex.toml", "error validating settings reconciler.matchers[0].conditions[0].type_: regex parse error:\n    foo(bar\n       ^\nerror: unclosed group reconciler.matchers[0].conditions[1].type_: regex parse error:\n    marco(polo\n         ^\nerror: unclosed group " ; "returns error on invalid condition type regex")]
    #[test_case("src/settings/testfiles/invalid_condition_status_regex.toml", "error validating settings reconciler.matchers[0].conditions[0].status: regex parse error:\n    foo(bar\n       ^\nerror: unclosed group reconciler.matchers[0].conditions[1].status: regex parse error:\n    marco(polo\n         ^\nerror: unclosed group " ; "returns error on invalid condition status regex")]
    fn new_tests(path: &str, expected_error: &str) {
        let res = Settings::new(path);
        assert!(res.is_err());
        assert!(res.err().unwrap().to_string().contains(expected_error));
    }

    #[test]
    fn new_returns_settings_on_valid_config() {
        let res = Settings::new("src/settings/testfiles/valid.toml");
        assert!(res.is_ok());
        let settings = res.unwrap();
        assert_eq!("0.0.0.0", settings.server.host);
        assert_eq!(8080, settings.server.port);
        assert_eq!(tracing::Level::INFO, settings.log.max_level);
        assert_eq!(1, settings.reconciler.matchers.len());
        let matcher = settings.reconciler.matchers.get(0).unwrap();
        assert_eq!(TaintEffect::NoExecute, matcher.taint.effect);
        assert_eq!("pressure", matcher.taint.key);
        assert_eq!("memory", matcher.taint.value);
        assert_eq!(2, matcher.conditions.len());
        let condition = matcher.conditions.get(0).unwrap();
        assert_eq!("NetworkInterfaceCard", condition.type_);
        assert_eq!("Kaput|Ruined", condition.status);
        let condition = matcher.conditions.get(1).unwrap();
        assert_eq!("PrivateLink", condition.type_);
        assert_eq!("severed", condition.status);
    }
}
