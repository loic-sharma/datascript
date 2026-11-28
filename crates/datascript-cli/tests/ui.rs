//! CLI integration tests for `ds ui` (M4-01).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use assert_cmd::Command as AssertCommand;
use predicates::prelude::*;
use tempfile::TempDir;

fn ds() -> AssertCommand {
    AssertCommand::new(assert_cmd::cargo::cargo_bin!("ds"))
}

#[test]
fn ui_help_describes_flags_and_sandbox() {
    ds().args(["ui", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--no-open"))
        .stdout(predicate::str::contains("--allow-outside-root"))
        .stdout(predicate::str::contains("--dev-proxy"))
        .stdout(predicate::str::contains("--max-rows"))
        .stdout(predicate::str::contains("127.0.0.1"));
}

#[test]
fn ui_rejects_missing_or_non_directory_root() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("a.csv");
    std::fs::write(&file, "x\n1\n").unwrap();

    ds().args(["ui", file.to_str().unwrap(), "--no-open"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("is not a directory"));

    let missing = dir.path().join("no_such_dir");
    ds().args(["ui", missing.to_str().unwrap(), "--no-open"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot open"));
}

#[test]
fn ui_starts_prints_tokenized_url_and_serves_api() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("items.csv"), "id,name\n1,apple\n").unwrap();

    let mut child = Command::new(assert_cmd::cargo::cargo_bin!("ds"))
        .args(["ui", dir.path().to_str().unwrap(), "--no-open"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ds ui");

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read startup line");

    let url = line
        .trim()
        .strip_prefix("DataScript UI listening on ")
        .unwrap_or_else(|| panic!("unexpected startup line: {line:?}"));
    let rest = url
        .strip_prefix("http://")
        .unwrap_or_else(|| panic!("expected http:// URL, got {url:?}"));
    let (host_port, query) = rest
        .split_once("/?token=")
        .unwrap_or_else(|| panic!("expected /?token= in URL, got {url:?}"));
    assert!(host_port.starts_with("127.0.0.1:"), "{host_port}");
    assert_eq!(query.len(), 64, "expected 64-char hex token: {query}");

    // Send a raw HTTP/1.1 GET /api/info with Bearer auth and verify 200 OK JSON.
    let mut stream = TcpStream::connect(host_port).expect("connect to ds ui");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "GET /api/info HTTP/1.1\r\nHost: {host_port}\r\nAuthorization: Bearer {query}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    let _ = child.kill();
    let _ = child.wait();

    assert!(
        response.starts_with("HTTP/1.1 200 OK\r\n"),
        "unexpected response: {response}"
    );
    assert!(
        response.contains("\"sandboxed\":true") && response.contains("\"defaultRowLimit\":10000"),
        "unexpected body: {response}"
    );
}
