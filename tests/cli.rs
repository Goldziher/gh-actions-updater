use serde_json::Value;
use std::fs;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_gh-actions-updater")
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
    assert!(stdout.contains("found 1 reference"));
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
fn unsupported_update_flag_exits_two() {
    let output = Command::new(binary()).arg("--update").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unsupported in this scanner iteration"));
}
