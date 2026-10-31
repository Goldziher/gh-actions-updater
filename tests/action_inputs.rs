#[cfg(unix)]
mod unix {
    use saphyr::{LoadableYamlNode, Yaml};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;

    fn run_script() -> String {
        let source = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("action.yml")).unwrap();
        let docs = Yaml::load_from_str(&source).unwrap();
        let steps = docs[0]["runs"]["steps"].as_vec().unwrap();
        steps.last().unwrap()["run"].as_str().unwrap().to_string()
    }

    fn invoke(inputs: &[(&str, &str)]) -> (Option<i32>, String) {
        let temp = tempfile::tempdir().unwrap();
        let fake = temp.path().join("gau");
        fs::write(&fake, "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\"\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let mut command = Command::new("bash");
        command
            .arg("-c")
            .arg(run_script())
            .env("GHAU_BIN", &fake)
            .env("INPUT_OPERATION", "check")
            .env("INPUT_MODE", "latest")
            .env("INPUT_PIN_STYLE", "preserve")
            .env("INPUT_PATH", ".")
            .env("INPUT_RECURSIVE", "false")
            .env("INPUT_VALIDATE", "true")
            .env("INPUT_MISSING_REF", "error");
        for (key, value) in inputs {
            command.env(key, value);
        }
        let output = command.output().unwrap();
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap().trim().to_string(),
        )
    }

    #[test]
    fn preserve_pin_style_passes_no_pin_style_flag() {
        assert_eq!(
            invoke(&[]),
            (Some(0), "--check --latest --missing-ref error --validate .".to_string())
        );
    }

    #[test]
    fn major_pin_style_is_forwarded() {
        assert_eq!(
            invoke(&[("INPUT_MODE", "latest-tag"), ("INPUT_PIN_STYLE", "major")]),
            (
                Some(0),
                "--check --latest-tag --missing-ref error --pin-style major --validate .".to_string()
            )
        );
    }

    #[test]
    fn lint_pins_operation_runs_offline_flag_only() {
        assert_eq!(
            invoke(&[("INPUT_OPERATION", "lint-pins"), ("INPUT_RECURSIVE", "true")]),
            (Some(0), "--lint-pins --recursive .".to_string())
        );
    }

    #[test]
    fn invalid_pin_style_is_rejected() {
        assert_eq!(invoke(&[("INPUT_PIN_STYLE", "sha")]).0, Some(2));
    }
}
