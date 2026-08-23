use saphyr::{LoadableYamlNode, Yaml};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

const CHECK_ARGUMENTS: &[&str] = &["--latest", "--check", "--validate", "--missing-ref", "error"];
const UPDATE_ARGUMENTS: &[&str] = &["--latest", "--update", "--validate", "--missing-ref", "error"];

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn yaml_value<'a>(yaml: &'a Yaml<'_>, key: &str) -> &'a Yaml<'a> {
    yaml.as_mapping_get(key)
        .unwrap_or_else(|| panic!("missing YAML key {key}"))
}

fn yaml_string<'a>(yaml: &'a Yaml<'_>, key: &str) -> &'a str {
    yaml_value(yaml, key)
        .as_str()
        .unwrap_or_else(|| panic!("YAML key {key} is not a string"))
}

#[test]
fn should_expose_check_update_and_legacy_pre_commit_hooks() {
    let content = fs::read_to_string(repository_root().join(".pre-commit-hooks.yaml")).unwrap();
    let documents = Yaml::load_from_str(&content).unwrap();
    let hooks = documents[0].as_sequence().unwrap();
    let expected = [
        ("gh-actions-updater-check", CHECK_ARGUMENTS),
        ("gh-actions-updater-update", UPDATE_ARGUMENTS),
        ("gh-actions-updater", UPDATE_ARGUMENTS),
    ];
    assert_eq!(hooks.len(), expected.len());

    for (expected_id, expected_arguments) in expected {
        let hook = hooks
            .iter()
            .find(|hook| yaml_string(hook, "id") == expected_id)
            .unwrap_or_else(|| panic!("missing pre-commit hook {expected_id}"));
        let entry = yaml_string(hook, "entry");
        assert_eq!(entry.split_whitespace().next(), Some("gau"));
        assert_eq!(entry.split_whitespace().skip(1).collect::<Vec<_>>(), expected_arguments);
        assert_eq!(yaml_value(hook, "pass_filenames").as_bool(), Some(false));
        assert_eq!(yaml_value(hook, "always_run").as_bool(), Some(true));
    }
}

#[derive(Debug, Deserialize)]
struct PolyManifest {
    version: u64,
    hooks: Vec<PolyHook>,
}

#[derive(Debug, Deserialize)]
struct PolyHook {
    id: String,
    args: Vec<String>,
    workspace: bool,
    pass_filenames: bool,
    always_run: bool,
    paths: Vec<PolyPath>,
}

#[derive(Debug, Deserialize)]
struct PolyPath {
    channel: String,
    run: String,
    install: Option<String>,
}

#[test]
fn should_expose_semantic_check_and_update_poly_hooks() {
    let content = fs::read_to_string(repository_root().join("poly-hooks.toml")).unwrap();
    let manifest: PolyManifest = toml::from_str(&content).unwrap();
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.hooks.len(), 2);

    for (expected_id, expected_arguments) in [
        ("gh-actions-updater-check", CHECK_ARGUMENTS),
        ("gh-actions-updater-update", UPDATE_ARGUMENTS),
    ] {
        let hook = manifest
            .hooks
            .iter()
            .find(|hook| hook.id == expected_id)
            .unwrap_or_else(|| panic!("missing Poly hook {expected_id}"));
        assert_eq!(hook.args, expected_arguments);
        assert!(hook.workspace);
        assert!(!hook.pass_filenames);
        assert!(hook.always_run);

        let system = hook.paths.iter().find(|path| path.channel == "system").unwrap();
        assert_eq!(system.run, "gau");
        assert_eq!(system.install, None);

        let cargo = hook.paths.iter().find(|path| path.channel == "cargo").unwrap();
        assert!(cargo.run.ends_with("/bin/gau\""));
        assert_eq!(
            cargo.install.as_deref(),
            Some("cargo install --locked gh-actions-updater --version 0.2.1")
        );
    }
}

#[test]
fn should_expose_bounded_validation_gate_github_action() {
    let content = fs::read_to_string(repository_root().join("action.yml")).unwrap();
    let documents = Yaml::load_from_str(&content).unwrap();
    let action = &documents[0];
    let inputs = yaml_value(action, "inputs");

    let input_names = inputs
        .as_mapping()
        .unwrap()
        .keys()
        .map(|key| key.as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        input_names,
        std::collections::BTreeSet::from([
            "cache",
            "github-token",
            "missing-ref",
            "mode",
            "operation",
            "path",
            "recursive",
            "validate",
            "version",
        ])
    );

    assert_eq!(yaml_string(yaml_value(inputs, "operation"), "default"), "check");
    assert_eq!(yaml_string(yaml_value(inputs, "mode"), "default"), "latest");
    assert_eq!(yaml_string(yaml_value(inputs, "validate"), "default"), "true");
    assert_eq!(yaml_string(yaml_value(inputs, "missing-ref"), "default"), "error");

    let runs = yaml_value(action, "runs");
    assert_eq!(yaml_string(runs, "using"), "composite");
    let steps = yaml_value(runs, "steps").as_sequence().unwrap();
    let cache_step = steps
        .iter()
        .find(|step| step.as_mapping_get("id").and_then(Yaml::as_str) == Some("cache"))
        .expect("missing cache step");
    assert_eq!(
        yaml_string(cache_step, "uses"),
        "actions/cache@0057852bfaa89a56745cba8c7296529d2fc39830"
    );
    let installer_step = steps
        .iter()
        .find(|step| {
            step.as_mapping_get("run")
                .and_then(Yaml::as_str)
                .is_some_and(|script| script.contains("scripts/install-action.sh"))
        })
        .expect("missing hardened installer step");
    assert_eq!(
        yaml_string(installer_step, "if"),
        "steps.cache.outputs.cache-hit != 'true'"
    );

    let operation_step = steps.last().unwrap();
    let environment = yaml_value(operation_step, "env");
    assert_eq!(
        yaml_string(environment, "GHAU_BIN"),
        "${{ steps.resolve.outputs.install-dir }}/${{ runner.os == 'Windows' && 'gau.exe' || 'gau' }}"
    );
    let execution_script = steps
        .iter()
        .filter_map(|step| step.as_mapping_get("run").and_then(Yaml::as_str))
        .find(|script| script.contains("INPUT_OPERATION"))
        .expect("missing gau execution step");

    assert!(execution_script.contains("case \"$INPUT_OPERATION\" in check|update)"));
    assert!(execution_script.contains("case \"$INPUT_MODE\" in latest|latest-tag|latest-hash)"));
    assert!(execution_script.contains("case \"$INPUT_MISSING_REF\" in warn|error|ignore|fallback)"));
    assert!(execution_script.contains("args=(\"--${INPUT_OPERATION}\" \"--${INPUT_MODE}\""));
    assert!(execution_script.contains("\"--missing-ref\" \"$INPUT_MISSING_REF\""));
    assert!(execution_script.contains("[ \"$INPUT_VALIDATE\" = \"true\" ] && args+=(\"--validate\")"));
}

#[test]
fn should_move_stable_major_tag_only_after_complete_release_is_published() {
    let content = fs::read_to_string(repository_root().join(".github/workflows/publish.yaml")).unwrap();
    let documents = Yaml::load_from_str(&content).unwrap();
    let jobs = yaml_value(&documents[0], "jobs");
    let finalize = yaml_value(jobs, "finalize_release");
    let needs = yaml_value(finalize, "needs").as_sequence().unwrap();
    let dependencies = needs.iter().map(|need| need.as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(dependencies, ["meta", "checksums"]);

    let steps = yaml_value(finalize, "steps").as_sequence().unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(
        yaml_string(&steps[0], "name"),
        "Promote draft to published release once complete"
    );
    let finalize_script = yaml_string(&steps[0], "run");
    for required_asset in [
        "gh-actions-updater-x86_64-unknown-linux-gnu.tar.gz",
        "gh-actions-updater-aarch64-unknown-linux-gnu.tar.gz",
        "gh-actions-updater-x86_64-apple-darwin.tar.gz",
        "gh-actions-updater-aarch64-apple-darwin.tar.gz",
        "gh-actions-updater-x86_64-pc-windows-gnu.zip",
        "checksums.txt",
    ] {
        assert!(
            finalize_script.contains(required_asset),
            "missing required asset {required_asset}"
        );
    }
    let completeness_gate = finalize_script.find("if [ \"$missing\" -gt 0 ]").unwrap();
    let publish_release = finalize_script.find("gh release edit \"$tag\" --draft=false").unwrap();
    assert!(completeness_gate < publish_release);

    assert_eq!(yaml_string(&steps[1], "name"), "Move stable major tag");
    assert_eq!(
        yaml_string(&steps[1], "if"),
        "${{ !contains(needs.meta.outputs.tag, '-') }}"
    );
    let stable_tag_script = yaml_string(&steps[1], "run");
    assert!(stable_tag_script.contains("git/refs/tags/v0"));
}
