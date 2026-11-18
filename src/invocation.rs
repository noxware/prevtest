//! Parses command-line arguments and environment variables into a test invocation.

use anyhow::{Context, Result, ensure};
use clap::{Parser, builder::BoolishValueParser};
use std::{env, ffi::OsString, path::PathBuf};

const DEFAULT_CACHE_DIR: &str = ".prevtest/cache";
// Separate from the cache directory, which CI may save and restore.
const DEFAULT_STATE_DIR: &str = ".prevtest/state";

#[derive(Parser)]
#[command(version, about = "Reuse previous test results with nextest.")]
pub struct Args {
    /// Execute again and replace any cached result for this key.
    // Match nextest's boolean environment variable parsing.
    #[arg(long, env = "PREVTEST_FORCE", value_parser = BoolishValueParser::new())]
    force: bool,

    /// Test executable followed by its arguments.
    #[arg(required = true, num_args = 1.., trailing_var_arg = true)]
    command: Vec<OsString>,
}

pub struct Invocation {
    pub executable: OsString,
    pub args: Vec<OsString>,
    pub binary_id: String,
    pub test_name: String,
    pub workspace: PathBuf,
    pub working_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub state_dir: PathBuf,
    pub force: bool,
    pub report_path: Option<PathBuf>,
}

impl Invocation {
    pub fn from_args(args: Args) -> Result<Self> {
        ensure!(
            env::var("NEXTEST_EXECUTION_MODE").as_deref() == Ok("process-per-test"),
            "prevtest must be configured as a nextest run wrapper"
        );
        let workspace = PathBuf::from(
            env::var_os("NEXTEST_WORKSPACE_ROOT").context("NEXTEST_WORKSPACE_ROOT is missing")?,
        );
        ensure!(workspace.is_absolute(), "workspace root must be absolute");
        let workspace = workspace
            .canonicalize()
            .context("resolving workspace root")?;
        // Relative directories resolve from the workspace root.
        let dir = |var, default: &str| workspace.join(env::var_os(var).unwrap_or(default.into()));
        let cache_dir = dir("PREVTEST_CACHE_DIR", DEFAULT_CACHE_DIR);
        let state_dir = dir("PREVTEST_STATE_DIR", DEFAULT_STATE_DIR);
        let mut command = args.command.into_iter();
        Ok(Self {
            executable: command.next().context("test executable is missing")?,
            args: command.collect(),
            binary_id: env::var("NEXTEST_BINARY_ID").context("NEXTEST_BINARY_ID is missing")?,
            test_name: env::var("NEXTEST_TEST_NAME").context("NEXTEST_TEST_NAME is missing")?,
            workspace,
            working_dir: env::current_dir()?,
            cache_dir,
            state_dir,
            force: args.force
                || env::var("NEXTEST_ATTEMPT").is_ok_and(|value| value != "1")
                || env::var("NEXTEST_STRESS_CURRENT").is_ok_and(|value| value != "none"),
            report_path: env::var_os("NEXTEST_RUN_WRAPPER_REPORT").map(PathBuf::from),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, FromArgMatches};

    fn parse(args: &[&str]) -> Result<Args, clap::Error> {
        let matches = Args::command()
            .mut_arg("force", |arg| arg.env(None::<&str>))
            .try_get_matches_from(args)?;
        Args::from_arg_matches(&matches)
    }

    #[test]
    fn force_is_a_flag_before_the_executable() {
        assert!(!parse(&["prevtest", "binary"]).unwrap().force);
        assert!(parse(&["prevtest", "--force", "binary"]).unwrap().force);
        assert!(parse(&["prevtest", "--force=true", "binary"]).is_err());
    }

    #[test]
    fn arguments_after_the_executable_are_passed_through() {
        let args = parse(&["prevtest", "binary", "--force", "--exact"]).unwrap();
        assert!(!args.force);
        assert_eq!(args.command, ["binary", "--force", "--exact"]);
    }
}
