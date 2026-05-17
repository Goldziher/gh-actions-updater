use crate::action_ref::{ParsedRef, ReferenceKind, parse_uses};
use crate::config::Settings;
use anyhow::{Context, Result};
use memchr::memmem;
use saphyr::{LoadableYamlNode, Mapping, Yaml};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct FileReport {
    pub path: String,
    pub kind: FileKind,
    pub references: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Workflow,
    ActionMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReferenceReport {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub raw: String,
    #[serde(flatten)]
    pub parsed: ParsedRef,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub file: String,
    pub line: Option<usize>,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct ScanOutput {
    pub files: Vec<FileReport>,
    pub references: Vec<ReferenceReport>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn scan_files(paths: &[PathBuf], settings: &Settings) -> Result<ScanOutput> {
    let mut output = ScanOutput::default();

    for path in paths {
        let kind = classify_file(path);
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let references = scan_content(path, kind, &content)?;
        for reference in &references {
            if reference.parsed.kind == ReferenceKind::Malformed {
                output.diagnostics.push(Diagnostic {
                    file: reference.file.clone(),
                    line: Some(reference.line),
                    message: format!("malformed or unsupported uses reference: {}", reference.raw),
                });
            }
        }

        output.files.push(FileReport {
            path: path.display().to_string(),
            kind,
            references: references.len(),
        });
        output.references.extend(references);

        if settings.schema_validation {
            output
                .diagnostics
                .extend(schema_diagnostics(path, kind, &content, settings));
        }
    }

    Ok(output)
}

fn classify_file(path: &Path) -> FileKind {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("action.yml" | "action.yaml") => FileKind::ActionMetadata,
        _ => FileKind::Workflow,
    }
}

fn scan_content(path: &Path, kind: FileKind, content: &str) -> Result<Vec<ReferenceReport>> {
    if memmem::find(content.as_bytes(), b"uses:").is_none() {
        return Ok(Vec::new());
    }

    let docs = Yaml::load_from_str(content)
        .with_context(|| format!("failed to parse YAML in {}", path.display()))?;
    let Some(yaml) = docs.first() else {
        return Ok(Vec::new());
    };

    let mut values = Vec::new();
    match kind {
        FileKind::Workflow => collect_workflow_uses(yaml, &mut values),
        FileKind::ActionMetadata => collect_action_metadata_uses(yaml, &mut values),
    }

    let mut references = Vec::new();
    let mut cursor = 0usize;
    for raw in values {
        let (line, column) = find_uses_location(content, &raw, &mut cursor).unwrap_or((0, 0));
        let parsed = parse_uses(&raw);
        references.push(ReferenceReport {
            file: path.display().to_string(),
            line,
            column,
            raw,
            parsed,
        });
    }
    Ok(references)
}

fn collect_workflow_uses(yaml: &Yaml<'_>, values: &mut Vec<String>) {
    let Some(jobs) = mapping_get(yaml, "jobs").and_then(Yaml::as_mapping) else {
        return;
    };

    for job in jobs.values().filter_map(Yaml::as_mapping) {
        collect_string_field(job, "uses", values);

        if let Some(steps) = mapping_get_value(job, "steps").and_then(Yaml::as_vec) {
            collect_step_uses(steps, values);
        }
    }
}

fn collect_action_metadata_uses(yaml: &Yaml<'_>, values: &mut Vec<String>) {
    let Some(runs) = mapping_get(yaml, "runs").and_then(Yaml::as_mapping) else {
        return;
    };
    let Some(using) = mapping_get_value(runs, "using").and_then(Yaml::as_str) else {
        return;
    };
    if !using.eq_ignore_ascii_case("composite") {
        return;
    }

    if let Some(steps) = mapping_get_value(runs, "steps").and_then(Yaml::as_vec) {
        collect_step_uses(steps, values);
    }
}

fn collect_step_uses(steps: &[Yaml<'_>], values: &mut Vec<String>) {
    for step in steps.iter().filter_map(Yaml::as_mapping) {
        collect_string_field(step, "uses", values);
    }
}

fn collect_string_field(mapping: &Mapping<'_>, key: &str, values: &mut Vec<String>) {
    if let Some(value) = mapping_get_value(mapping, key) {
        if let Some(value) = value.as_str() {
            values.push(value.to_string());
        } else {
            values.push(String::new());
        }
    }
}

fn mapping_get<'a>(value: &'a Yaml<'_>, key: &str) -> Option<&'a Yaml<'a>> {
    value.as_mapping_get(key)
}

fn mapping_get_value<'a>(mapping: &'a Mapping<'_>, key: &str) -> Option<&'a Yaml<'a>> {
    mapping
        .iter()
        .find(|(mapping_key, _)| mapping_key.as_str() == Some(key))
        .map(|(_, value)| value)
}

fn find_uses_location(content: &str, raw: &str, cursor: &mut usize) -> Option<(usize, usize)> {
    for (line_index, line) in content.lines().enumerate().skip(*cursor) {
        if line.contains("uses:") && (raw.is_empty() || line.contains(raw)) {
            *cursor = line_index + 1;
            return Some((line_index + 1, line.find("uses").unwrap_or(0) + 1));
        }
    }

    for (line_index, line) in content.lines().enumerate() {
        if line.contains("uses:") && (raw.is_empty() || line.contains(raw)) {
            *cursor = line_index + 1;
            return Some((line_index + 1, line.find("uses").unwrap_or(0) + 1));
        }
    }

    None
}

fn schema_diagnostics(
    path: &Path,
    kind: FileKind,
    content: &str,
    settings: &Settings,
) -> Vec<Diagnostic> {
    if content.trim().is_empty() {
        return vec![Diagnostic {
            file: path.display().to_string(),
            line: None,
            message: "empty GitHub Actions YAML file".to_string(),
        }];
    }

    let mut diagnostics = Vec::new();
    if kind == FileKind::ActionMetadata && !content.contains("runs:") {
        diagnostics.push(Diagnostic {
            file: path.display().to_string(),
            line: None,
            message: "action metadata is missing a runs section".to_string(),
        });
    }
    if settings.strict_schema {
        diagnostics.push(Diagnostic {
            file: path.display().to_string(),
            line: None,
            message: "strict schema validation is not implemented in this scanner iteration"
                .to_string(),
        });
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::{FileKind, scan_content};
    use crate::action_ref::ReferenceKind;
    use std::path::Path;

    #[test]
    fn scans_workflow_step_and_reusable_workflow_refs() {
        let content = r#"
jobs:
  test:
    uses: org/reuse/.github/workflows/ci.yml@v1
  build:
    steps:
      - uses: actions/checkout@v4
      - uses: docker://alpine:3
"#;
        let refs = scan_content(
            Path::new(".github/workflows/ci.yml"),
            FileKind::Workflow,
            content,
        )
        .unwrap();
        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].parsed.kind, ReferenceKind::ReusableWorkflow);
        assert_eq!(refs[1].parsed.kind, ReferenceKind::RemoteAction);
        assert_eq!(refs[2].parsed.kind, ReferenceKind::DockerImage);
    }

    #[test]
    fn scans_composite_action_metadata() {
        let content = r#"
runs:
  using: composite
  steps:
    - uses: actions/setup-node@v4
"#;
        let refs =
            scan_content(Path::new("action.yml"), FileKind::ActionMetadata, content).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].raw, "actions/setup-node@v4");
    }

    #[test]
    fn skips_non_composite_action_metadata_uses() {
        let content = r#"
runs:
  using: node20
  main: dist/index.js
"#;
        let refs =
            scan_content(Path::new("action.yml"), FileKind::ActionMetadata, content).unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn marks_expression_refs_as_malformed() {
        let content = r#"
jobs:
  test:
    steps:
      - uses: ${{ inputs.action }}
"#;
        let refs = scan_content(
            Path::new(".github/workflows/ci.yml"),
            FileKind::Workflow,
            content,
        )
        .unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].parsed.kind, ReferenceKind::Malformed);
    }

    #[test]
    fn ignores_unsupported_uses_locations() {
        let content = r#"
uses: top/level@v1
jobs:
  test:
    steps:
      - run: echo test
        with:
          uses: nested/data@v1
"#;
        let refs = scan_content(
            Path::new(".github/workflows/ci.yml"),
            FileKind::Workflow,
            content,
        )
        .unwrap();
        assert!(refs.is_empty());
    }
}
