//! Coordinates the test caching workflow.

use crate::{
    execution::{self, CapturedOutput},
    identity::{self, CacheKey},
    invocation::Invocation,
    reporting::{self, Outcome},
    storage::{Cache, State},
};
use anyhow::{Context, Result};
use std::process::ExitStatus;

pub fn run(invocation: Invocation) -> Result<ExitStatus> {
    let binary_path = invocation.working_dir.join(&invocation.executable);
    let state = State::open(&invocation.state_dir)
        .inspect_err(|error| eprintln!("prevtest: local state unavailable: {error:#}"))
        .ok();
    let binary_hash = identity::file_hash(&binary_path, state.as_ref());
    // Release the state before running the test, which may take a while.
    drop(state);
    let binary_hash = match binary_hash {
        Ok(hash) => hash,
        Err(error) => return on_cache_unavailable(&invocation, error),
    };
    let key = CacheKey::new(
        &binary_hash,
        &invocation.binary_id,
        &invocation.test_name,
        &invocation.working_dir,
        &invocation.workspace,
        &invocation.args,
    );
    let cache = match Cache::open(&invocation.cache_dir) {
        Ok(cache) => cache,
        Err(error) => return on_cache_unavailable(&invocation, error),
    };

    if invocation.force {
        return on_force(&invocation, &cache, &key);
    }

    let cached_result = cache
        .get_result(&key)
        .with_context(|| format!("loading cached result for {}", binary_path.display()));
    match cached_result {
        Ok(Some(output)) => on_cache_hit(&invocation, &output),
        Ok(None) => on_cache_miss(&invocation, &cache, &key),
        Err(error) => {
            drop(cache);
            on_cache_unavailable(&invocation, error)
        }
    }
}

fn on_force(invocation: &Invocation, cache: &Cache, key: &CacheKey) -> Result<ExitStatus> {
    let result = execute_and_update_cache(invocation, cache, key)?;
    match result.cache_update {
        Ok(()) => {
            reporting::write(invocation.report_path.as_deref(), Outcome::Forced);
            Ok(result.status)
        }
        Err(error) => on_cache_update_failed(invocation, error, result.status),
    }
}

fn on_cache_hit(invocation: &Invocation, output: &CapturedOutput) -> Result<ExitStatus> {
    execution::replay(output)?;
    reporting::write(invocation.report_path.as_deref(), Outcome::Cached);
    Ok(ExitStatus::default())
}

fn on_cache_miss(invocation: &Invocation, cache: &Cache, key: &CacheKey) -> Result<ExitStatus> {
    let result = execute_and_update_cache(invocation, cache, key)?;
    match result.cache_update {
        Ok(()) => {
            reporting::write(invocation.report_path.as_deref(), Outcome::CacheMiss);
            Ok(result.status)
        }
        Err(error) => on_cache_update_failed(invocation, error, result.status),
    }
}

fn on_cache_unavailable(invocation: &Invocation, error: anyhow::Error) -> Result<ExitStatus> {
    eprintln!("prevtest: cache unavailable, executing test: {error:#}");
    let executed = execution::run(&invocation.executable, &invocation.args)?;
    reporting::write(invocation.report_path.as_deref(), Outcome::CacheUnavailable);
    Ok(executed.status)
}

fn on_cache_update_failed(
    invocation: &Invocation,
    error: anyhow::Error,
    status: ExitStatus,
) -> Result<ExitStatus> {
    eprintln!("prevtest: could not update cache: {error:#}");
    reporting::write(
        invocation.report_path.as_deref(),
        Outcome::CacheUpdateFailed,
    );
    Ok(status)
}

struct RunResult {
    status: ExitStatus,
    cache_update: Result<()>,
}

fn execute_and_update_cache(
    invocation: &Invocation,
    cache: &Cache,
    key: &CacheKey,
) -> Result<RunResult> {
    let executed = execution::run(&invocation.executable, &invocation.args)?;
    let cache_update = if executed.status.success() && !executed.interrupted {
        cache.put_result(
            key,
            &invocation.binary_id,
            &invocation.test_name,
            &executed.output,
        )
    } else {
        cache.delete_result(key)
    };
    Ok(RunResult {
        status: executed.status,
        cache_update,
    })
}
