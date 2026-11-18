use std::{fs, path::Path, process::Command};

#[test]
fn cache_lifecycle() {
    let project = tempfile::tempdir().unwrap();
    let mut cmd = nextest(project.path());
    let executions = project.path().join("executions");
    let greeting = project.path().join("greeting.txt");

    // Cold cache, then both results reused.
    check_run(&mut cmd, &executions, true, 1, 1);
    check_run(&mut cmd, &executions, true, 1, 1);

    // Force both tests, with one failure.
    fs::write(&greeting, "goodbye\n").unwrap();
    cmd.env("PREVTEST_FORCE", "1");
    check_run(&mut cmd, &executions, false, 2, 2);

    // Rerun the failed test and reuse the other.
    fs::write(&greeting, "hello\n").unwrap();
    cmd.env_remove("PREVTEST_FORCE");
    check_run(&mut cmd, &executions, true, 3, 2);

    // A changed assertion rebuilds the binary and invalidates both cached results.
    let source = project.path().join("src/lib.rs");
    let code = fs::read_to_string(&source).unwrap();
    fs::write(&source, code.replace("\"hello\"", "\"goodbye\"")).unwrap();
    check_run(&mut cmd, &executions, false, 4, 3);
}

#[test]
fn signal_and_timeout() {
    let project = tempfile::tempdir().unwrap();
    let mut cmd = nextest(project.path());
    fs::write(
        project.path().join("src/lib.rs"),
        include_str!("fixtures/signals.rs"),
    )
    .unwrap();
    let config_path = project.path().join("nextest.toml");
    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str(
        r#"
[[profile.default.overrides]]
filter = 'test(ignores_term) | test(handles_term)'
slow-timeout = { period = "1s", terminate-after = 1, grace-period = "500ms" }
"#,
    );
    fs::write(config_path, config).unwrap();

    let output = cmd.output().unwrap();
    let log = String::from_utf8(output.stderr).unwrap();
    assert!(!output.status.success(), "{log}");
    assert!(
        log.lines()
            .any(|line| line.contains("SIGTERM") && line.contains("dies_by_signal")),
        "{log}"
    );
    assert!(
        log.lines()
            .any(|line| line.contains("TIMEOUT") && line.contains("ignores_term")),
        "{log}"
    );
    // The test handles cancellation by exiting 0. A cache hit would skip its cleanup.
    cmd.args(["-E", "test(handles_term)"]).output().unwrap();
    let cleanups = fs::read_to_string(project.path().join("cleanups")).unwrap();
    assert_eq!(cleanups.lines().count(), 2);

    // A killed child may still be listed as a zombie until init reaps it.
    let pid = fs::read_to_string(project.path().join("sleeper.pid")).unwrap();
    let ps = Command::new("ps")
        .args(["-o", "stat=", "-p", pid.trim()])
        .output()
        .unwrap();
    let state = String::from_utf8(ps.stdout).unwrap();
    assert!(
        state.trim().is_empty() || state.trim().starts_with('Z'),
        "test {} survived timeout: {state}",
        pid.trim()
    );
}

fn nextest(project: &Path) -> Command {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project");
    fs::create_dir(project.join("src")).unwrap();
    for file in [
        "Cargo.toml",
        "Cargo.lock",
        "src/lib.rs",
        "greeting.txt",
        "number.txt",
    ] {
        fs::copy(fixture.join(file), project.join(file)).unwrap();
    }
    let config = project.join("nextest.toml");

    fs::write(
        &config,
        format!(
            include_str!("fixtures/nextest.toml.in"),
            prevtest = env!("CARGO_BIN_EXE_prevtest")
        ),
    )
    .unwrap();

    let mut cmd = Command::new(env!("CARGO"));
    cmd.current_dir(project)
        .args([
            "nextest",
            "run",
            "--offline",
            "--locked",
            "--profile",
            "default",
            "--config-file",
        ])
        .arg(&config)
        .args(["--no-fail-fast", "--success-output", "immediate"])
        .env("CARGO_TARGET_DIR", project.join("target"))
        .env_remove("PREVTEST_CACHE_DIR")
        .env_remove("PREVTEST_STATE_DIR")
        .env_remove("PREVTEST_FORCE");

    cmd
}

#[track_caller]
fn check_run(
    cmd: &mut Command,
    execution_log: &Path,
    success: bool,
    greeting_runs: usize,
    number_runs: usize,
) {
    let output = cmd.output().unwrap();
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.success(), success, "{log}");
    assert!(log.contains("number: 42"), "{log}");
    assert!(log.contains("checking number"), "{log}");
    let executions = fs::read_to_string(execution_log).unwrap();
    for (name, expected) in [
        ("reads_greeting", greeting_runs),
        ("reads_number", number_runs),
    ] {
        let actual = executions.lines().filter(|line| *line == name).count();
        assert_eq!(actual, expected, "executions of {name}: {log}");
    }
}
