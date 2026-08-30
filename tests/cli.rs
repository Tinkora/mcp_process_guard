use assert_cmd::Command;
use predicates::prelude::*;

#[test]
#[cfg(unix)]
fn no_handshake_reports_clean_exit_without_echoing_arguments() {
    let mut command = Command::cargo_bin("mcp-process-guard").unwrap();
    command
        .args([
            "--no-handshake",
            "--output",
            "json",
            "--",
            "sh",
            "-c",
            "exit 0",
            "token=secret-value",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""outcome":"exited""#))
        .stdout(predicate::str::contains("secret-value").not());
}

#[test]
#[cfg(unix)]
fn timeout_is_bounded_and_does_not_echo_arguments() {
    let mut command = Command::cargo_bin("mcp-process-guard").unwrap();
    command
        .args([
            "--no-handshake",
            "--grace-ms",
            "30",
            "--",
            "sh",
            "-c",
            "sleep 5",
            "--api-key=visible-no",
        ])
        .assert()
        .code(3)
        .stdout(predicate::str::contains("TimedOut"))
        .stdout(predicate::str::contains("visible-no").not());
}

#[test]
#[cfg(unix)]
fn invalid_handshake_fails_and_cleans_up() {
    let mut command = Command::cargo_bin("mcp-process-guard").unwrap();
    command
        .args([
            "--handshake-timeout-ms",
            "200",
            "--",
            "sh",
            "-c",
            "printf 'not-json\\n'; sleep 5",
        ])
        .assert()
        .code(predicate::in_iter([4, 7]))
        .stdout(predicate::str::contains("HandshakeFailed"));
}

#[test]
#[cfg(unix)]
fn oversized_handshake_frame_is_rejected_with_bounded_cleanup() {
    let mut command = Command::cargo_bin("mcp-process-guard").unwrap();
    command
        .args([
            "--max-handshake-bytes",
            "32",
            "--cleanup-timeout-ms",
            "200",
            "--",
            "sh",
            "-c",
            "printf '%080d\\n' 0; sleep 5",
        ])
        .assert()
        .code(predicate::in_iter([4, 7]))
        .stdout(predicate::str::contains("HandshakeFailed"));
}

#[test]
#[cfg(unix)]
fn valid_handshake_is_reported_before_clean_exit() {
    let mut command = Command::cargo_bin("mcp-process-guard").unwrap();
    command
        .args([
            "--output",
            "json",
            "--",
            "sh",
            "-c",
            r#"read request; printf '{"jsonrpc":"2.0","id":1,"result":{}}\n'; read initialized"#,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""handshake":"succeeded""#));
}

#[test]
#[cfg(unix)]
fn reports_and_cleans_owned_descendant_after_root_exits() {
    let mut command = Command::cargo_bin("mcp-process-guard").unwrap();
    command
        .args([
            "--no-handshake",
            "--grace-ms",
            "500",
            "--output",
            "json",
            "--",
            "sh",
            "-c",
            "sleep 30 &",
        ])
        .assert()
        .code(6)
        .stdout(predicate::str::contains(
            r#""outcome":"descendants_survived""#,
        ))
        .stdout(predicate::str::contains(r#""cleanup":"succeeded""#));
}

#[test]
fn rejects_zero_and_excessive_timeouts() {
    Command::cargo_bin("mcp-process-guard")
        .unwrap()
        .args(["--no-handshake", "--grace-ms", "0", "--", "ignored"])
        .assert()
        .code(2);
    Command::cargo_bin("mcp-process-guard")
        .unwrap()
        .args(["--no-handshake", "--grace-ms", "3600001", "--", "ignored"])
        .assert()
        .code(2);
}

#[test]
fn json_spawn_failure_is_structured_and_redacted() {
    let mut command = Command::cargo_bin("mcp-process-guard").unwrap();
    command
        .args([
            "--no-handshake",
            "--output",
            "json",
            "--",
            "definitely-not-a-real-command-secret-value",
        ])
        .assert()
        .code(5)
        .stdout(predicate::str::contains(r#""outcome":"spawn_failed""#))
        .stdout(predicate::str::contains("secret-value").not());
}

#[test]
#[cfg(windows)]
fn windows_no_handshake_reports_clean_exit() {
    let mut command = Command::cargo_bin("mcp-process-guard").unwrap();
    command
        .args([
            "--no-handshake",
            "--output",
            "json",
            "--",
            "cmd",
            "/C",
            "exit",
            "0",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""outcome":"exited""#));
}

#[test]
#[cfg(windows)]
fn windows_job_reports_and_cleans_descendant_after_root_exits() {
    let mut command = Command::cargo_bin("mcp-process-guard").unwrap();
    command
        .args([
            "--no-handshake",
            "--output",
            "json",
            "--",
            "cmd",
            "/C",
            "start /B ping -n 30 127.0.0.1 >NUL & exit /B 0",
        ])
        .assert()
        .code(6)
        .stdout(predicate::str::contains(
            r#""outcome":"descendants_survived""#,
        ))
        .stdout(predicate::str::contains(r#""cleanup":"succeeded""#));
}
