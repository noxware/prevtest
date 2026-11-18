use std::{fs, path::Path, process::Command};

#[test]
fn cache_lifecycle() {
    let project = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/project");
    fs::create_dir(project.path().join("src")).unwrap();
    for file in [
        "Cargo.toml",
        "Cargo.lock",
        "src/lib.rs",
        "greeting.txt",
        "number.txt",
    ] {
        fs::copy(fixture.join(file), project.path().join(file)).unwrap();
    }
    let config = project.path().join("nextest.toml");
    let executions = project.path().join("executions");
    let greeting = project.path().join("greeting.txt");

    fs::write(
        &config,
        format!(
            include_str!("fixtures/nextest.toml.in"),
            prevtest = env!("CARGO_BIN_EXE_prevtest")
        ),
    )
    .unwrap();

    let mut cmd = Command::new(env!("CARGO"));
    cmd.current_dir(project.path())
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
        .env("CARGO_TARGET_DIR", project.path().join("target"))
        .env_remove("PREVTEST_CACHE_DIR")
        .env_remove("PREVTEST_STATE_DIR")
        .env_remove("PREVTEST_FORCE");

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
