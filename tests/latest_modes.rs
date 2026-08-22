use serde_json::Value;
use std::fs;
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_gau")
}

fn local_repository() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let workflow = temp.path().join(".github/workflows/ci.yml");
    let action = temp.path().join(".github/actions/local/action.yml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::create_dir_all(action.parent().unwrap()).unwrap();
    fs::write(
        workflow,
        "jobs:\n  test:\n    steps:\n      - uses: ./.github/actions/local\n",
    )
    .unwrap();
    fs::write(action, "name: local\nruns:\n  using: composite\n  steps: []\n").unwrap();
    temp
}

fn run_local(arguments: &[&str]) -> Output {
    let temp = local_repository();
    Command::new(binary())
        .args(arguments)
        .arg("--no-cache")
        .arg("--no-schema-validation")
        .arg(temp.path())
        .output()
        .unwrap()
}

#[test]
fn should_accept_each_latest_mode() {
    for mode in ["--latest-tag", "--latest-hash", "--latest"] {
        let output = run_local(&[mode]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{mode} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn should_reject_multiple_latest_modes() {
    for modes in [
        ["--latest-tag", "--latest-hash"],
        ["--latest-tag", "--latest"],
        ["--latest-hash", "--latest"],
    ] {
        let output = run_local(&modes);
        assert_eq!(output.status.code(), Some(2), "modes {modes:?} were accepted");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("cannot be used with"),
            "unexpected error for {modes:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn should_accept_all_update_modes_from_config() {
    for mode in ["latest-tag", "latest-hash", "latest"] {
        let temp = local_repository();
        let config = temp.path().join("config.toml");
        fs::write(&config, format!("[update]\nmode = \"{mode}\"\n")).unwrap();

        let output = Command::new(binary())
            .arg("--config")
            .arg(config)
            .arg("--no-cache")
            .arg("--no-schema-validation")
            .arg(temp.path())
            .output()
            .unwrap();

        assert_eq!(
            output.status.code(),
            Some(0),
            "config mode {mode} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn should_allow_cli_latest_hash_with_configured_tag_pin_style() {
    let temp = local_repository();
    let config = temp.path().join("config.toml");
    fs::write(&config, "[update]\nmode = \"latest-tag\"\npin_style = \"major\"\n").unwrap();

    let output = Command::new(binary())
        .arg("--config")
        .arg(config)
        .arg("--latest-hash")
        .arg("--no-cache")
        .arg("--no-schema-validation")
        .arg(temp.path())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "CLI mode did not override config pin style: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn should_allow_cli_latest_tag_to_override_configured_latest_hash() {
    let temp = local_repository();
    let config = temp.path().join("config.toml");
    fs::write(&config, "[update]\nmode = \"latest-hash\"\n").unwrap();

    let output = Command::new(binary())
        .arg("--config")
        .arg(config)
        .arg("--latest-tag")
        .arg("--pin-style")
        .arg("full")
        .arg("--no-cache")
        .arg("--no-schema-validation")
        .arg(temp.path())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "CLI latest-tag did not override config: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn should_resolve_local_action_from_repository_root_when_scanning_github_directory() {
    let temp = local_repository();
    let output = Command::new(binary())
        .arg("--validate")
        .arg("--format")
        .arg("json")
        .arg("--no-cache")
        .arg("--no-schema-validation")
        .arg(temp.path().join(".github"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));

    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["references_found"], 1);
    assert_eq!(report["diagnostics"], serde_json::json!([]));
}

#[test]
fn should_report_structured_skip_and_diagnostic_counts() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = temp.path().join(".github/workflows/ci.yml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::write(
        workflow,
        "jobs:\n  test:\n    steps:\n      - uses: actions/checkout@main\n      - uses: ./missing\n",
    )
    .unwrap();

    let output = Command::new(binary())
        .arg("--validate")
        .arg("--format")
        .arg("json")
        .arg("--no-cache")
        .arg("--no-schema-validation")
        .arg(temp.path())
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(report["summary"]["skipped"], 2);
    assert_eq!(report["summary"]["failures"], 1);
    assert_eq!(report["diagnostics"][0]["category"], "validation");
    assert_eq!(report["diagnostics"][0]["code"], "local_reference_missing");
    assert_eq!(report["skips"][0]["code"], "floating_ref_requires_opt_in");
    assert_eq!(report["skips"][1]["code"], "local_reference");
}
