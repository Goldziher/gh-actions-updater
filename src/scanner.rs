use crate::action_ref::{ParsedRef, ReferenceKind, parse_uses};
use crate::config::Settings;
use anyhow::{Context, Result};
use memchr::memmem;
use saphyr::{AnnotatedMapping, LoadableYamlNode, MarkedYaml, Scalar, Yaml};
use serde::Serialize;
use serde_json::{Map, Number, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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
    #[serde(skip)]
    pub ref_span: Option<ByteSpan>,
    #[serde(skip)]
    pub rewrite_supported: bool,
    #[serde(skip)]
    pub rewrite_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ByteSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub file: String,
    pub line: Option<usize>,
    pub message: String,
    #[serde(skip)]
    pub category: DiagnosticCategory,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum DiagnosticCategory {
    #[default]
    General,
    Schema,
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
                    category: DiagnosticCategory::General,
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

pub fn has_schema_diagnostics(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.category == DiagnosticCategory::Schema)
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

    let docs = MarkedYaml::load_from_str(content)
        .with_context(|| format!("failed to parse YAML in {}", path.display()))?;
    let Some(yaml) = docs.first() else {
        return Ok(Vec::new());
    };

    let mut values = Vec::new();
    match kind {
        FileKind::Workflow => collect_workflow_uses(yaml, content, &mut values),
        FileKind::ActionMetadata => collect_action_metadata_uses(yaml, content, &mut values),
    }

    let mut references = Vec::new();
    for value in values {
        let raw = value.raw;
        let parsed = parse_uses(&raw);
        let ref_span = value
            .value_span
            .and_then(|span| find_ref_span(&raw, span.start));
        let rewrite_supported = ref_span.is_some() && parsed.ref_name.is_some();
        let rewrite_reason = if rewrite_supported {
            None
        } else {
            Some("uses value is not a simple single-line rewrite target".to_string())
        };
        references.push(ReferenceReport {
            file: path.display().to_string(),
            line: value.line,
            column: value.column,
            raw,
            parsed,
            ref_span,
            rewrite_supported,
            rewrite_reason,
        });
    }
    Ok(references)
}

#[derive(Debug, Clone)]
struct UseValue {
    raw: String,
    line: usize,
    column: usize,
    value_span: Option<ByteSpan>,
}

fn collect_workflow_uses(yaml: &MarkedYaml<'_>, content: &str, values: &mut Vec<UseValue>) {
    let Some(jobs) = mapping_get_marked(yaml, "jobs").and_then(|value| value.data.as_mapping())
    else {
        return;
    };

    for job in jobs.values().filter_map(|value| value.data.as_mapping()) {
        collect_string_field(job, "uses", content, values);

        if let Some(steps) =
            mapping_get_value_marked(job, "steps").and_then(|value| value.data.as_vec())
        {
            collect_step_uses(steps, content, values);
        }
    }
}

fn collect_action_metadata_uses(yaml: &MarkedYaml<'_>, content: &str, values: &mut Vec<UseValue>) {
    let Some(runs) = mapping_get_marked(yaml, "runs").and_then(|value| value.data.as_mapping())
    else {
        return;
    };
    let Some(using) = mapping_get_value_marked(runs, "using").and_then(|value| value.data.as_str())
    else {
        return;
    };
    if !using.eq_ignore_ascii_case("composite") {
        return;
    }

    if let Some(steps) =
        mapping_get_value_marked(runs, "steps").and_then(|value| value.data.as_vec())
    {
        collect_step_uses(steps, content, values);
    }
}

fn collect_step_uses(steps: &[MarkedYaml<'_>], content: &str, values: &mut Vec<UseValue>) {
    for step in steps.iter().filter_map(|value| value.data.as_mapping()) {
        collect_string_field(step, "uses", content, values);
    }
}

fn collect_string_field(
    mapping: &AnnotatedMapping<'_, MarkedYaml<'_>>,
    key: &str,
    content: &str,
    values: &mut Vec<UseValue>,
) {
    if let Some((key_node, value_node)) = mapping_get_pair_marked(mapping, key) {
        let raw = value_node
            .data
            .as_str()
            .map(str::to_string)
            .unwrap_or_default();
        let value_span = find_value_span(value_node, &raw)
            .and_then(|span| find_value_span_in_source(content, span, &raw));
        values.push(UseValue {
            raw,
            line: key_node.span.start.line(),
            column: key_node.span.start.col(),
            value_span,
        });
    }
}

fn mapping_get_marked<'a>(value: &'a MarkedYaml<'_>, key: &str) -> Option<&'a MarkedYaml<'a>> {
    value.data.as_mapping_get(key)
}

fn mapping_get_value_marked<'a>(
    mapping: &'a AnnotatedMapping<'_, MarkedYaml<'_>>,
    key: &str,
) -> Option<&'a MarkedYaml<'a>> {
    mapping_get_pair_marked(mapping, key).map(|(_, value)| value)
}

fn mapping_get_pair_marked<'a>(
    mapping: &'a AnnotatedMapping<'_, MarkedYaml<'_>>,
    key: &str,
) -> Option<(&'a MarkedYaml<'a>, &'a MarkedYaml<'a>)> {
    mapping
        .iter()
        .find(|(mapping_key, _)| mapping_key.data.as_str() == Some(key))
}

fn find_value_span(value: &MarkedYaml<'_>, raw: &str) -> Option<ByteSpan> {
    let start = value.span.start.index();
    let end = value.span.end.index();
    if raw.is_empty() || start >= end {
        return None;
    }
    Some(ByteSpan { start, end })
}

fn find_ref_span(raw: &str, raw_start: usize) -> Option<ByteSpan> {
    let (_, ref_name) = raw.rsplit_once('@')?;
    let ref_start_in_raw = raw.len().checked_sub(ref_name.len())?;
    Some(ByteSpan {
        start: raw_start + ref_start_in_raw,
        end: raw_start + raw.len(),
    })
}

fn find_value_span_in_source(content: &str, parser_span: ByteSpan, raw: &str) -> Option<ByteSpan> {
    if parser_span.end > content.len()
        || !content.is_char_boundary(parser_span.start)
        || !content.is_char_boundary(parser_span.end)
    {
        return None;
    }
    let source = &content[parser_span.start..parser_span.end];
    let offset = source.find(raw)?;
    Some(ByteSpan {
        start: parser_span.start + offset,
        end: parser_span.start + offset + raw.len(),
    })
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
            category: DiagnosticCategory::Schema,
        }];
    }

    let mut diagnostics = Vec::new();
    let docs = match Yaml::load_from_str(content) {
        Ok(docs) => docs,
        Err(error) => {
            diagnostics.push(Diagnostic {
                file: path.display().to_string(),
                line: None,
                message: format!("failed to parse YAML for schema validation: {error}"),
                category: DiagnosticCategory::Schema,
            });
            return diagnostics;
        }
    };
    let Some(yaml) = docs.first() else {
        return diagnostics;
    };
    let json = yaml_to_json(yaml);
    let validator = match schema_validator(kind) {
        Ok(validator) => validator,
        Err(error) => {
            diagnostics.push(Diagnostic {
                file: path.display().to_string(),
                line: None,
                message: format!("failed to compile vendored schema: {error}"),
                category: DiagnosticCategory::Schema,
            });
            return diagnostics;
        }
    };
    for error in validator.iter_errors(&json).take(8) {
        diagnostics.push(Diagnostic {
            file: path.display().to_string(),
            line: None,
            message: format!(
                "schema validation failed at {}: {error}",
                error.instance_path()
            ),
            category: DiagnosticCategory::Schema,
        });
    }
    if settings.strict_schema && !diagnostics.is_empty() {
        diagnostics.push(Diagnostic {
            file: path.display().to_string(),
            line: None,
            message: "strict schema validation failed".to_string(),
            category: DiagnosticCategory::Schema,
        });
    }
    diagnostics
}

fn schema_validator(kind: FileKind) -> &'static Result<jsonschema::Validator, String> {
    static WORKFLOW: OnceLock<Result<jsonschema::Validator, String>> = OnceLock::new();
    static ACTION: OnceLock<Result<jsonschema::Validator, String>> = OnceLock::new();

    match kind {
        FileKind::Workflow => {
            WORKFLOW.get_or_init(|| compile_schema(include_str!("schemas/github-workflow.json")))
        }
        FileKind::ActionMetadata => {
            ACTION.get_or_init(|| compile_schema(include_str!("schemas/github-action.json")))
        }
    }
}

fn compile_schema(schema: &str) -> Result<jsonschema::Validator, String> {
    let schema: Value =
        serde_json::from_str(schema).map_err(|error| format!("failed to load schema: {error}"))?;
    jsonschema::validator_for(&schema).map_err(|error| error.to_string())
}

fn yaml_to_json(yaml: &Yaml<'_>) -> Value {
    match yaml {
        Yaml::Representation(value, _, _) => Value::String(value.to_string()),
        Yaml::Value(Scalar::Null) => Value::Null,
        Yaml::Value(Scalar::Boolean(value)) => Value::Bool(*value),
        Yaml::Value(Scalar::Integer(value)) => Value::Number(Number::from(*value)),
        Yaml::Value(Scalar::FloatingPoint(value)) => Number::from_f64(value.into_inner())
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Yaml::Value(Scalar::String(value)) => Value::String(value.to_string()),
        Yaml::Sequence(values) => Value::Array(values.iter().map(yaml_to_json).collect()),
        Yaml::Mapping(mapping) => {
            let mut object = Map::new();
            for (key, value) in mapping {
                let key = key
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| match key {
                        Yaml::Value(Scalar::Integer(value)) => value.to_string(),
                        Yaml::Value(Scalar::Boolean(value)) => value.to_string(),
                        _ => serde_json::to_string(&yaml_to_json(key)).unwrap_or_default(),
                    });
                object.insert(key, yaml_to_json(value));
            }
            Value::Object(object)
        }
        Yaml::Tagged(_, value) => yaml_to_json(value),
        Yaml::Alias(_) | Yaml::BadValue => Value::Null,
    }
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
    fn rewrite_span_is_parser_bounded_not_comment_match() {
        let content = r#"
jobs:
  test:
    steps:
      # uses: actions/checkout@v4
      - uses: actions/checkout@v4
"#;
        let refs = scan_content(
            Path::new(".github/workflows/ci.yml"),
            FileKind::Workflow,
            content,
        )
        .unwrap();
        let span = refs[0].ref_span.unwrap();

        assert_eq!(&content[span.start..span.end], "v4");
        assert!(span.start > content.find("# uses:").unwrap());
    }

    #[test]
    fn rewrite_span_handles_quoted_scalar_value() {
        let content = r#"
jobs:
  test:
    steps:
      - uses: "actions/checkout@v4"
"#;
        let refs = scan_content(
            Path::new(".github/workflows/ci.yml"),
            FileKind::Workflow,
            content,
        )
        .unwrap();
        let span = refs[0].ref_span.unwrap();

        assert_eq!(&content[span.start..span.end], "v4");
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
