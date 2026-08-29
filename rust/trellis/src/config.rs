//! `soil.toml` per `docs/soil-toml.md` (approved 2026-08-25): strict
//! parsing (unknown keys and premature reserved sections are errors),
//! the v1 surface (`entrypoint`, `[toolchain]`, `[tags]`), root
//! discovery, and the refuse-on-mismatch toolchain pin (design §8).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::diag::ErrorReport;

/// The soil0 CLI contract major this toolchain drives
/// (docs/contracts/soil0-cli.md §1).
pub const SOIL0_CLI_MAJOR: i64 = 1;

pub fn trellis_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Reserved section names: declaring one before its milestone is an
/// error so its arrival stays additive (soil-toml §3).
const RESERVED_SECTIONS: [&str; 5] = ["trusted_packages", "deps", "build", "ci", "providers"];

#[derive(Debug, Clone)]
pub struct Toolchain {
    pub trellis: String,
    pub soil0_cli: i64,
    pub skill: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub entrypoint: Option<String>,
    pub toolchain: Toolchain,
    pub tags: BTreeMap<String, String>,
}

/// The nearest ancestor of `start` containing a `soil.toml` (design
/// §7.2: a Soil root is any directory with one at its top).
/// Root discovery: `TRELLIS_ROOT` wins when set (the cram runner
/// exports it so transcripts in temp dirs can call back into their
/// root — resolved 2026-08-28, step 8), else walk up from `start`
/// for a `soil.toml`.
pub fn find_root(start: &Path) -> Result<PathBuf, ErrorReport> {
    if let Some(root) = std::env::var_os("TRELLIS_ROOT") {
        let root = PathBuf::from(root);
        if root.join("soil.toml").is_file() {
            return Ok(root);
        }
        return Err(ErrorReport::one(
            "config-no-root",
            format!(
                "TRELLIS_ROOT={} has no soil.toml at its top",
                root.display()
            ),
        ));
    }
    let mut dir = start.to_path_buf();
    loop {
        if dir.join("soil.toml").is_file() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(ErrorReport::one(
                "config-no-root",
                format!(
                    "no soil.toml found in {} or any ancestor; run inside a Soil root",
                    start.display()
                ),
            ));
        }
    }
}

pub fn load(root: &Path) -> Result<Config, ErrorReport> {
    let path = root.join("soil.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| {
        ErrorReport::one("config-io", format!("cannot read {}: {e}", path.display()))
    })?;
    parse(&text)
}

fn parse(text: &str) -> Result<Config, ErrorReport> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| ErrorReport::one("config-parse", format!("soil.toml: {e}")))?;

    let mut entrypoint = None;
    let mut toolchain = None;
    let mut tags = BTreeMap::new();

    for (key, value) in &table {
        match key.as_str() {
            "entrypoint" => {
                entrypoint = Some(expect_string(value, "entrypoint")?);
            }
            "toolchain" => {
                toolchain = Some(parse_toolchain(expect_table(value, "toolchain")?)?);
            }
            "tags" => {
                for (tag, desc) in expect_table(value, "tags")? {
                    tags.insert(tag.clone(), expect_string(desc, &format!("tags.{tag}"))?);
                }
            }
            reserved if RESERVED_SECTIONS.contains(&reserved) => {
                return Err(ErrorReport::one(
                    "config-reserved-section",
                    format!(
                        "soil.toml section [{reserved}] is reserved for a later milestone \
                         (docs/soil-toml.md §3) and cannot be declared yet"
                    ),
                ));
            }
            unknown => {
                return Err(ErrorReport::one(
                    "config-unknown-key",
                    format!("soil.toml: unknown key `{unknown}`"),
                ));
            }
        }
    }

    let toolchain = toolchain.ok_or_else(|| {
        ErrorReport::one("config-bad-value", "soil.toml: missing [toolchain] table")
    })?;

    Ok(Config {
        entrypoint,
        toolchain,
        tags,
    })
}

fn parse_toolchain(table: &toml::Table) -> Result<Toolchain, ErrorReport> {
    let mut trellis = None;
    let mut soil0_cli = None;
    let mut skill = None;

    for (key, value) in table {
        match key.as_str() {
            "trellis" => trellis = Some(expect_string(value, "toolchain.trellis")?),
            "soil0_cli" => soil0_cli = Some(expect_integer(value, "toolchain.soil0_cli")?),
            "skill" => skill = Some(expect_string(value, "toolchain.skill")?),
            unknown => {
                return Err(ErrorReport::one(
                    "config-unknown-key",
                    format!("soil.toml: unknown key `toolchain.{unknown}`"),
                ));
            }
        }
    }

    Ok(Toolchain {
        trellis: trellis.ok_or_else(|| {
            ErrorReport::one(
                "config-bad-value",
                "soil.toml: toolchain.trellis is required",
            )
        })?,
        soil0_cli: soil0_cli.ok_or_else(|| {
            ErrorReport::one(
                "config-bad-value",
                "soil.toml: toolchain.soil0_cli is required",
            )
        })?,
        skill,
    })
}

/// Refuse-on-mismatch (design §8, soil-toml §2.1). A missing `skill`
/// pin only warns in the pre-generator window; from impl step 12 it
/// refuses (resolved 2026-08-25).
pub fn check_pin(config: &Config) -> Result<(), ErrorReport> {
    let mut problems = Vec::new();
    if config.toolchain.trellis != trellis_version() {
        problems.push(format!(
            "soil.toml pins trellis {} but this binary is {}",
            config.toolchain.trellis,
            trellis_version()
        ));
    }
    if config.toolchain.soil0_cli != SOIL0_CLI_MAJOR {
        problems.push(format!(
            "soil.toml pins soil0_cli {} but this toolchain speaks {}",
            config.toolchain.soil0_cli, SOIL0_CLI_MAJOR
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(ErrorReport::one(
            "toolchain-mismatch",
            format!(
                "{}; run `trellis toolchain update` to adopt the running toolchain",
                problems.join("; ")
            ),
        ))
    }
}

fn expect_string(value: &toml::Value, at: &str) -> Result<String, ErrorReport> {
    value.as_str().map(str::to_string).ok_or_else(|| {
        ErrorReport::one(
            "config-bad-value",
            format!("soil.toml: `{at}` must be a string"),
        )
    })
}

fn expect_integer(value: &toml::Value, at: &str) -> Result<i64, ErrorReport> {
    value.as_integer().ok_or_else(|| {
        ErrorReport::one(
            "config-bad-value",
            format!("soil.toml: `{at}` must be an integer"),
        )
    })
}

fn expect_table<'v>(value: &'v toml::Value, at: &str) -> Result<&'v toml::Table, ErrorReport> {
    value.as_table().ok_or_else(|| {
        ErrorReport::one(
            "config-bad-value",
            format!("soil.toml: `{at}` must be a table"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> String {
        format!(
            "entrypoint = \"main\"\n\n[toolchain]\ntrellis = \"{}\"\nsoil0_cli = {}\n\n\
             [tags]\napi = \"public surface\"\n",
            trellis_version(),
            SOIL0_CLI_MAJOR
        )
    }

    #[test]
    fn parses_and_pin_checks() {
        let config = parse(&good()).unwrap();
        assert_eq!(config.entrypoint.as_deref(), Some("main"));
        assert_eq!(config.tags["api"], "public surface");
        check_pin(&config).unwrap();
    }

    #[test]
    fn unknown_key_rejected() {
        let err = parse("[toolchain]\ntrellis = \"0\"\nsoil0_cli = 1\nbanana = 2\n").unwrap_err();
        assert_eq!(err.errors[0].code, "config-unknown-key");
        let err = parse(&format!("{}\n[banana]\n", good())).unwrap_err();
        assert_eq!(err.errors[0].code, "config-unknown-key");
    }

    #[test]
    fn reserved_section_rejected() {
        let err = parse(&format!("{}\n[deps]\n", good())).unwrap_err();
        assert_eq!(err.errors[0].code, "config-reserved-section");
    }

    #[test]
    fn missing_toolchain_rejected() {
        let err = parse("entrypoint = \"main\"\n").unwrap_err();
        assert_eq!(err.errors[0].code, "config-bad-value");
    }

    #[test]
    fn pin_mismatch_refuses() {
        let config =
            parse(&good().replace(&format!("soil0_cli = {SOIL0_CLI_MAJOR}"), "soil0_cli = 99"))
                .unwrap();
        let err = check_pin(&config).unwrap_err();
        assert_eq!(err.errors[0].code, "toolchain-mismatch");
    }
}
