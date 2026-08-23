use serde_json::Value;
use std::fs;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_gau")
}

#[test]
fn human_scan_exits_zero() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = temp.path().join(".github/workflows/ci.yml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::write(
        &workflow,
        r#"
jobs:
  test:
    steps:
      - uses: ./.github/actions/local
"#,
    )
    .unwrap();

    let output = Command::new(binary())
        .arg("--no-cache")
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("found 1 reference(s)"));
    assert!(stdout.contains("./.github/actions/local"));
}

#[test]
fn json_output_keeps_top_level_shape_when_quiet() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = temp.path().join(".github/workflows/ci.yml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::write(
        &workflow,
        r#"
jobs:
  call:
    uses: ./.github/workflows/reuse.yml
"#,
    )
    .unwrap();

    let output = Command::new(binary())
        .arg("--quiet")
        .arg("--no-cache")
        .arg("--format")
        .arg("json")
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["changed"], false);
    assert_eq!(value["would_change"], false);
    assert_eq!(value["summary"]["files_scanned"], 1);
    assert_eq!(value["summary"]["references_found"], 1);
    assert_eq!(value["summary"]["updates_available"], 0);
    assert!(value["files"].is_array());
    assert!(value["references"].is_array());
    assert!(value["updates"].is_array());
    assert!(value["diagnostics"].is_array());
    assert!(value["cache"].is_object());
}

#[test]
fn init_writes_config_and_requires_force_for_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join(".gh-actions-updater.toml");

    let output = Command::new(binary())
        .arg("--init")
        .arg("--output")
        .arg(&config)
        .output()
        .unwrap();
    assert!(output.status.success());
    let content = fs::read_to_string(&config).unwrap();
    assert!(content.contains("[scan]"));
    assert!(content.contains("recursive = false"));
    assert!(content.contains("pin_style = \"preserve\""));
    assert!(content.contains("preserve_major = false"));

    let output = Command::new(binary())
        .arg("--init")
        .arg("--output")
        .arg(&config)
        .output()
        .unwrap();
    assert!(!output.status.success());

    let output = Command::new(binary())
        .arg("--init")
        .arg("--recursive")
        .arg("--force")
        .arg("--output")
        .arg(&config)
        .output()
        .unwrap();
    assert!(output.status.success());
    let content = fs::read_to_string(&config).unwrap();
    assert!(content.contains("recursive = true"));
}

#[test]
fn latest_hash_flag_is_supported() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = temp.path().join(".github/workflows/ci.yml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::write(
        &workflow,
        r#"
jobs:
  test:
    steps:
      - uses: ./.github/actions/local
"#,
    )
    .unwrap();

    let output = Command::new(binary())
        .arg("--latest-hash")
        .arg("--no-cache")
        .arg("--no-schema-validation")
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn pin_style_flag_is_supported() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = temp.path().join(".github/workflows/ci.yml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::write(
        &workflow,
        r#"
jobs:
  test:
    steps:
      - uses: ./.github/actions/local
"#,
    )
    .unwrap();

    let output = Command::new(binary())
        .arg("--pin-style")
        .arg("major")
        .arg("--no-cache")
        .arg("--no-schema-validation")
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn human_color_options_are_honored() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = temp.path().join(".github/workflows/ci.yml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::write(
        &workflow,
        r#"
jobs:
  test:
    steps:
      - uses: ./.github/actions/local
"#,
    )
    .unwrap();

    let output = Command::new(binary())
        .arg("--color")
        .arg("never")
        .arg("--no-cache")
        .arg("--no-schema-validation")
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("\x1b["));

    let output = Command::new(binary())
        .arg("--color")
        .arg("always")
        .arg("--no-cache")
        .arg("--no-schema-validation")
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\x1b["));
}

#[test]
fn check_validate_fails_for_missing_local_reference() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = temp.path().join(".github/workflows/ci.yml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::write(
        workflow,
        "jobs:\n  test:\n    steps:\n      - uses: ./.github/actions/missing\n",
    )
    .unwrap();

    let output = Command::new(binary())
        .args([
            "--check",
            "--validate",
            "--format",
            "json",
            "--no-cache",
            "--no-schema-validation",
        ])
        .arg(temp.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["diagnostics"][0]["code"], "local_reference_missing");
}

#[test]
fn check_validate_fails_for_local_action_directory_without_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = temp.path().join(".github/workflows/ci.yml");
    let empty_action = temp.path().join(".github/actions/empty");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::create_dir_all(empty_action).unwrap();
    fs::write(
        workflow,
        "jobs:\n  test:\n    steps:\n      - uses: ./.github/actions/empty\n",
    )
    .unwrap();

    let output = Command::new(binary())
        .args([
            "--check",
            "--validate",
            "--format",
            "json",
            "--no-cache",
            "--no-schema-validation",
        ])
        .arg(temp.path())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["diagnostics"][0]["code"], "local_reference_missing");
}

#[test]
fn json_reports_stable_skip_codes_without_counting_valid_local_or_docker_as_failures() {
    let temp = tempfile::tempdir().unwrap();
    let workflow = temp.path().join(".github/workflows/ci.yml");
    let local_action = temp.path().join(".github/actions/local/action.yml");
    let config = temp.path().join("config.toml");
    fs::create_dir_all(workflow.parent().unwrap()).unwrap();
    fs::create_dir_all(local_action.parent().unwrap()).unwrap();
    fs::write(local_action, "name: local\nruns:\n  using: composite\n  steps: []\n").unwrap();
    fs::write(
        workflow,
        concat!(
            "jobs:\n  test:\n    steps:\n",
            "      - uses: actions/checkout@main # gau: ignore\n",
            "      - uses: owner/excluded@main\n",
            "      - uses: ./.github/actions/local\n",
            "      - uses: docker://alpine:3\n",
        ),
    )
    .unwrap();
    fs::write(&config, "[update]\nexclude = [\"owner/excluded\"]\n").unwrap();

    let output = Command::new(binary())
        .args([
            "--config",
            config.to_str().unwrap(),
            "--format",
            "json",
            "--no-cache",
            "--no-schema-validation",
        ])
        .arg(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let codes: Vec<_> = report["skips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skip| skip["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"inline_ignore"));
    assert!(codes.contains(&"update_excluded"));
    assert!(codes.contains(&"local_reference"));
    assert!(codes.contains(&"docker_reference"));
    assert_eq!(report["summary"]["failures"], 0);
}
