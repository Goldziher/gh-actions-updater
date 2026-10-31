use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_gau")
}

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn lint_pins(root: &Path, extra: &[&str]) -> Output {
    Command::new(binary())
        .current_dir(root)
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_TOKEN")
        .env_remove("GHAU_GITHUB_TOKEN")
        .arg("--lint-pins")
        .arg("--github-api-url")
        .arg("http://127.0.0.1:9")
        .args(extra)
        .arg(".")
        .output()
        .unwrap()
}

fn bad_fixture(root: &Path) {
    fs::create_dir_all(root.join(".git")).unwrap();
    write(
        &root.join(".github/workflows/ci.yml"),
        &format!(
            r#"on: push
jobs:
  call:
    uses: octo-org/ci/.github/workflows/reuse.yml@v1.2.3
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@{SHA} # v6
      - uses: actions/setup-node@v4.1
      - uses: actions/cache@v4.2.0
      - uses: dtolnay/rust-toolchain@stable
      - uses: github/codeql-action/init@main
      - uses: actions/setup-python@v5
      - uses: ./.github/actions/local
      - uses: docker://alpine:3
      - uses: actions/upload-artifact@main # gau: ignore
"#
        ),
    );
    write(
        &root.join(".github/actions/build/action.yml"),
        r#"name: build
description: build
runs:
  using: composite
  steps:
    - uses: actions/download-artifact@v4.1.8
    - uses: actions/github-script@v7
"#,
    );
}

#[test]
fn lint_pins_reports_every_non_major_ref_and_exits_one() {
    let temp = tempfile::tempdir().unwrap();
    bad_fixture(temp.path());

    let output = lint_pins(temp.path(), &[]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let workflow = Path::new(".github/workflows/ci.yml").display().to_string();
    let action = Path::new(".github/actions/build/action.yml").display().to_string();
    let lines: Vec<&str> = stdout.lines().filter(|line| line.contains(": ")).collect();
    assert_eq!(
        lines,
        vec![
            format!("./{action}:6: actions/download-artifact@v4.1.8: full version tag; pin the major tag v4").as_str(),
            format!(
                "./{workflow}:4: octo-org/ci/.github/workflows/reuse.yml@v1.2.3: full version tag; pin the major tag v1"
            )
            .as_str(),
            format!("./{workflow}:8: actions/checkout@{SHA}: commit SHA; pin a vN major tag").as_str(),
            format!("./{workflow}:9: actions/setup-node@v4.1: full version tag; pin the major tag v4").as_str(),
            format!("./{workflow}:10: actions/cache@v4.2.0: full version tag; pin the major tag v4").as_str(),
            format!("./{workflow}:11: dtolnay/rust-toolchain@stable: branch or non-version ref; pin a vN major tag")
                .as_str(),
            format!("./{workflow}:12: github/codeql-action/init@main: branch or non-version ref; pin a vN major tag")
                .as_str(),
        ],
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("7 pin finding(s) in 10 remote reference(s) across 2 file(s)"),
        "stdout: {stdout}"
    );
}

#[test]
fn lint_pins_clean_fixture_exits_zero_with_zero_findings() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".git")).unwrap();
    write(
        &temp.path().join(".github/workflows/ci.yml"),
        r#"on: push
jobs:
  call:
    uses: octo-org/ci/.github/workflows/reuse.yml@v1
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: ./.github/actions/local
      - uses: docker://alpine:3
"#,
    );

    let output = lint_pins(temp.path(), &[]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("0 pin finding(s) in 2 remote reference(s) across 1 file(s)"),
        "stdout: {stdout}"
    );
}

#[test]
fn lint_pins_allow_full_permits_full_tags_but_not_shas_or_branches() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".git")).unwrap();
    write(
        &temp.path().join(".gh-actions-updater.toml"),
        "[update]\nallow_full = [\"rust-lang/crates-io-auth-action\", \"octo-org/*\"]\n",
    );
    write(
        &temp.path().join(".github/workflows/ci.yml"),
        &format!(
            r#"on: push
jobs:
  call:
    uses: octo-org/ci/.github/workflows/reuse.yml@v1.2.3
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: rust-lang/crates-io-auth-action@v1.0.6
      - uses: rust-lang/crates-io-auth-action@{SHA}
      - uses: octo-org/tool/sub@main
"#
        ),
    );

    let output = lint_pins(temp.path(), &[]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("crates-io-auth-action@v1.0.6"), "stdout: {stdout}");
    assert!(!stdout.contains("reuse.yml@v1.2.3"), "stdout: {stdout}");
    assert!(
        stdout.contains(&format!("rust-lang/crates-io-auth-action@{SHA}: commit SHA")),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("octo-org/tool/sub@main: branch or non-version ref"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("2 pin finding(s) in 4 remote reference(s)"),
        "stdout: {stdout}"
    );
}

#[test]
fn lint_pins_json_lists_findings_with_counts() {
    let temp = tempfile::tempdir().unwrap();
    bad_fixture(temp.path());

    let output = lint_pins(temp.path(), &["--format", "json"]);

    assert_eq!(output.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["summary"]["files_scanned"], 2);
    assert_eq!(value["summary"]["references_checked"], 10);
    assert_eq!(value["summary"]["findings"], 7);
    let findings = value["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 7);
    let sha = findings
        .iter()
        .find(|finding| finding["kind"] == "sha")
        .expect("sha finding");
    assert_eq!(sha["uses"], format!("actions/checkout@{SHA}"));
    assert_eq!(sha["ref"], SHA);
    assert_eq!(sha["line"], 8);
    assert!(sha["file"].as_str().unwrap().ends_with("ci.yml"));
    assert!(sha["reason"].as_str().unwrap().contains("commit SHA"));
    assert_eq!(
        findings.iter().filter(|finding| finding["kind"] == "full_tag").count(),
        4
    );
    assert_eq!(
        findings
            .iter()
            .filter(|finding| finding["kind"] == "branch_or_other")
            .count(),
        2
    );
}

#[test]
fn lint_pins_rejects_update_flags() {
    let temp = tempfile::tempdir().unwrap();
    bad_fixture(temp.path());

    let output = lint_pins(temp.path(), &["--update"]);

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn lint_pins_exits_three_on_unparseable_yaml() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".git")).unwrap();
    write(
        &temp.path().join(".github/workflows/ci.yml"),
        "jobs:\n  test:\n    steps:\n      - uses: [actions/checkout@v6\n",
    );

    let output = lint_pins(temp.path(), &[]);

    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to parse YAML"));
}

#[test]
fn lint_pins_accepts_an_unprefixed_major_it_would_suggest() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".git")).unwrap();
    write(
        &temp.path().join(".github/workflows/ci.yml"),
        r#"on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: octo-org/plain@1.2.3
      - uses: octo-org/plain@1
"#,
    );

    let output = lint_pins(temp.path(), &[]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("octo-org/plain@1.2.3: full version tag; pin the major tag 1\n"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains("octo-org/plain@1:"), "stdout: {stdout}");
    assert!(
        stdout.contains("1 pin finding(s) in 2 remote reference(s)"),
        "stdout: {stdout}"
    );
}

#[test]
fn lint_pins_scans_every_yaml_document() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".git")).unwrap();
    write(
        &temp.path().join(".github/workflows/ci.yml"),
        r#"on: push
jobs:
  first:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
---
on: push
jobs:
  second:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/cache@v4.2.0
"#,
    );

    let output = lint_pins(temp.path(), &[]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains(":13: actions/cache@v4.2.0: full version tag"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("1 pin finding(s) in 2 remote reference(s)"),
        "stdout: {stdout}"
    );
}

#[test]
fn lint_pins_reports_an_aliased_step_once() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".git")).unwrap();
    write(
        &temp.path().join(".github/workflows/ci.yml"),
        r#"on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - &checkout
        uses: actions/checkout@main
      - *checkout
"#,
    );

    let output = lint_pins(temp.path(), &[]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.matches("actions/checkout@main").count(), 1, "stdout: {stdout}");
    assert!(
        stdout.contains("1 pin finding(s) in 1 remote reference(s)"),
        "stdout: {stdout}"
    );
}
