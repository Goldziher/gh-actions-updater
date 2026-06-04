use serde::Serialize;

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    RemoteAction,
    ReusableWorkflow,
    LocalAction,
    LocalWorkflow,
    DockerImage,
    Malformed,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    SemverLikeTag,
    Sha,
    BranchOrUnknown,
    Branch,
    NonSemverTag,
    None,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct ParsedRef {
    pub kind: ReferenceKind,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub path: Option<String>,
    pub ref_name: Option<String>,
    pub ref_kind: RefKind,
    pub updatable: bool,
}

pub fn parse_uses(value: &str) -> ParsedRef {
    let value = value.trim();
    if value.is_empty() {
        return malformed();
    }

    if value.starts_with("./") || value.starts_with("../") {
        return ParsedRef {
            kind: if value.contains(".github/workflows/") {
                ReferenceKind::LocalWorkflow
            } else {
                ReferenceKind::LocalAction
            },
            owner: None,
            repo: None,
            path: Some(value.to_string()),
            ref_name: None,
            ref_kind: RefKind::None,
            updatable: false,
        };
    }

    if value.starts_with("docker://") {
        return ParsedRef {
            kind: ReferenceKind::DockerImage,
            owner: None,
            repo: None,
            path: Some(value.to_string()),
            ref_name: None,
            ref_kind: RefKind::None,
            updatable: false,
        };
    }

    let Some((target, ref_name)) = value.rsplit_once('@') else {
        return malformed();
    };
    let mut segments = target.split('/');
    let Some(owner) = segments.next().filter(|segment| !segment.is_empty()) else {
        return malformed();
    };
    let Some(repo) = segments.next().filter(|segment| !segment.is_empty()) else {
        return malformed();
    };
    let rest = segments.collect::<Vec<_>>().join("/");
    if ref_name.is_empty() {
        return malformed();
    }

    let kind = if rest.starts_with(".github/workflows/") {
        ReferenceKind::ReusableWorkflow
    } else {
        ReferenceKind::RemoteAction
    };

    let ref_kind = classify_ref(ref_name);
    ParsedRef {
        kind,
        owner: Some(owner.to_string()),
        repo: Some(repo.to_string()),
        path: if rest.is_empty() { None } else { Some(rest) },
        ref_name: Some(ref_name.to_string()),
        updatable: ref_kind == RefKind::SemverLikeTag,
        ref_kind,
    }
}

fn malformed() -> ParsedRef {
    ParsedRef {
        kind: ReferenceKind::Malformed,
        owner: None,
        repo: None,
        path: None,
        ref_name: None,
        ref_kind: RefKind::None,
        updatable: false,
    }
}

fn classify_ref(ref_name: &str) -> RefKind {
    if ref_name.len() == 40 && ref_name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return RefKind::Sha;
    }
    if is_semver_like(ref_name) {
        return RefKind::SemverLikeTag;
    }
    RefKind::BranchOrUnknown
}

fn is_semver_like(ref_name: &str) -> bool {
    let version = ref_name.strip_prefix('v').unwrap_or(ref_name);
    let parts: Vec<_> = version.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return false;
    }
    parts
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::{ReferenceKind, parse_uses};

    #[test]
    fn parses_remote_action() {
        let parsed = parse_uses("actions/checkout@v4");
        assert_eq!(parsed.kind, ReferenceKind::RemoteAction);
        assert_eq!(parsed.owner.as_deref(), Some("actions"));
        assert_eq!(parsed.repo.as_deref(), Some("checkout"));
        assert_eq!(parsed.ref_name.as_deref(), Some("v4"));
        assert!(parsed.updatable);
    }

    #[test]
    fn parses_reusable_workflow() {
        let parsed = parse_uses("octo-org/ci/.github/workflows/reuse.yml@v1");
        assert_eq!(parsed.kind, ReferenceKind::ReusableWorkflow);
        assert_eq!(parsed.path.as_deref(), Some(".github/workflows/reuse.yml"));
    }

    #[test]
    fn classifies_local_and_docker_refs() {
        assert_eq!(
            parse_uses("./.github/actions/build").kind,
            ReferenceKind::LocalAction
        );
        assert_eq!(
            parse_uses("./.github/workflows/reuse.yml").kind,
            ReferenceKind::LocalWorkflow
        );
        assert_eq!(
            parse_uses("docker://alpine:3").kind,
            ReferenceKind::DockerImage
        );
    }

    #[test]
    fn only_semver_like_remote_refs_are_default_updatable() {
        assert!(parse_uses("actions/checkout@v4.1.0").updatable);
        assert!(!parse_uses("actions/checkout@main").updatable);
        assert!(!parse_uses("actions/checkout@0123456789abcdef0123456789abcdef01234567").updatable);
        assert!(!parse_uses("actions/checkout@release-2026-05-17").updatable);
    }
}
