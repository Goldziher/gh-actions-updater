#[cfg(unix)]
mod unix {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    fn repository_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn write_executable(path: &Path, content: &str) {
        fs::write(path, content).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    struct InstallerFixture {
        _temp: tempfile::TempDir,
        fake_bin: PathBuf,
        archive: PathBuf,
        checksums: PathBuf,
        install_dir: PathBuf,
    }

    struct WindowsInstallerFixture {
        _temp: tempfile::TempDir,
        fake_bin: PathBuf,
        archive: PathBuf,
        checksums: PathBuf,
        binary: PathBuf,
        install_dir: PathBuf,
    }

    impl InstallerFixture {
        fn new(checksum_matches: bool) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let fake_bin = temp.path().join("fake-bin");
            let payload = temp.path().join("payload");
            let install_dir = temp.path().join("installed");
            fs::create_dir_all(&fake_bin).unwrap();
            fs::create_dir_all(&payload).unwrap();
            write_executable(&payload.join("gau"), "#!/usr/bin/env bash\necho fixture-gau\n");

            let archive_name = "gh-actions-updater-x86_64-unknown-linux-gnu.tar.gz";
            let archive = temp.path().join(archive_name);
            let status = Command::new("tar")
                .args(["-czf"])
                .arg(&archive)
                .arg("-C")
                .arg(&payload)
                .arg("gau")
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(0));

            let digest_output = Command::new("shasum")
                .args(["-a", "256"])
                .arg(&archive)
                .output()
                .unwrap();
            assert_eq!(digest_output.status.code(), Some(0));
            let digest = String::from_utf8(digest_output.stdout)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .to_string();
            let expected = if checksum_matches { digest } else { "0".repeat(64) };
            let checksums = temp.path().join("checksums.txt");
            fs::write(&checksums, format!("{expected}  {archive_name}\n")).unwrap();

            write_executable(
                &fake_bin.join("uname"),
                "#!/usr/bin/env bash\ncase \"$1\" in -s) echo \"${FAKE_UNAME_S:-Linux}\" ;; -m) echo \"${FAKE_UNAME_M:-x86_64}\" ;; esac\n",
            );
            write_executable(
                &fake_bin.join("curl"),
                r#"#!/usr/bin/env bash
set -euo pipefail
url=""
output=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    http*) url="$1"; shift ;;
    *) shift ;;
  esac
done
case "$url" in
  */checksums.txt) cp "$FIXTURE_CHECKSUMS" "$output" ;;
  *) cp "$FIXTURE_ARCHIVE" "$output" ;;
esac
"#,
            );

            Self {
                _temp: temp,
                fake_bin,
                archive,
                checksums,
                install_dir,
            }
        }

        fn run(&self, operating_system: &str, architecture: &str) -> Output {
            let system_path = std::env::var("PATH").unwrap();
            Command::new("bash")
                .arg(repository_root().join("scripts/install-action.sh"))
                .arg("0.2.0")
                .env("PATH", format!("{}:{system_path}", self.fake_bin.display()))
                .env("GHAU_INSTALL_DIR", &self.install_dir)
                .env("FIXTURE_ARCHIVE", &self.archive)
                .env("FIXTURE_CHECKSUMS", &self.checksums)
                .env("FAKE_UNAME_S", operating_system)
                .env("FAKE_UNAME_M", architecture)
                .output()
                .unwrap()
        }
    }

    impl WindowsInstallerFixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let fake_bin = temp.path().join("fake-bin");
            let install_dir = temp.path().join("installed");
            fs::create_dir_all(&fake_bin).unwrap();
            let archive_name = "gh-actions-updater-x86_64-pc-windows-gnu.zip";
            let archive = temp.path().join(archive_name);
            fs::write(&archive, b"hermetic zip fixture").unwrap();
            let binary = temp.path().join("gau.exe");
            fs::write(&binary, b"fixture windows executable").unwrap();
            let digest = sha256(&archive);
            let checksums = temp.path().join("checksums.txt");
            fs::write(&checksums, format!("{digest}  {archive_name}\n")).unwrap();
            write_executable(
                &fake_bin.join("uname"),
                "#!/usr/bin/env bash\ncase \"$1\" in -s) echo MSYS_NT-10.0 ;; -m) echo x86_64 ;; esac\n",
            );
            write_executable(&fake_bin.join("curl"), FAKE_CURL);
            write_executable(
                &fake_bin.join("unzip"),
                "#!/usr/bin/env bash\nset -euo pipefail\nwhile [ \"$1\" != \"-d\" ]; do shift; done\nmkdir -p \"$2\"\ncp \"$FIXTURE_BINARY\" \"$2/gau.exe\"\n",
            );
            Self {
                _temp: temp,
                fake_bin,
                archive,
                checksums,
                binary,
                install_dir,
            }
        }

        fn run(&self) -> Output {
            let system_path = std::env::var("PATH").unwrap();
            Command::new("bash")
                .arg(repository_root().join("scripts/install-action.sh"))
                .arg("0.2.0")
                .env("PATH", format!("{}:{system_path}", self.fake_bin.display()))
                .env("GHAU_INSTALL_DIR", &self.install_dir)
                .env("FIXTURE_ARCHIVE", &self.archive)
                .env("FIXTURE_CHECKSUMS", &self.checksums)
                .env("FIXTURE_BINARY", &self.binary)
                .output()
                .unwrap()
        }
    }

    const FAKE_CURL: &str = r#"#!/usr/bin/env bash
set -euo pipefail
url=""
output=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    http*) url="$1"; shift ;;
    *) shift ;;
  esac
done
case "$url" in
  */checksums.txt) cp "$FIXTURE_CHECKSUMS" "$output" ;;
  *) cp "$FIXTURE_ARCHIVE" "$output" ;;
esac
"#;

    fn sha256(path: &Path) -> String {
        let output = Command::new("shasum").args(["-a", "256"]).arg(path).output().unwrap();
        assert_eq!(output.status.code(), Some(0));
        String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    }

    #[test]
    fn should_install_binary_after_checksum_verification() {
        let fixture = InstallerFixture::new(true);
        let output = fixture.run("Linux", "x86_64");

        assert_eq!(
            output.status.code(),
            Some(0),
            "installer failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let installed = fixture.install_dir.join("gau");
        assert_eq!(
            fs::read_to_string(&installed).unwrap(),
            "#!/usr/bin/env bash\necho fixture-gau\n"
        );
        assert_eq!(fs::metadata(installed).unwrap().permissions().mode() & 0o111, 0o111);
    }

    #[test]
    fn should_reject_archive_when_checksum_does_not_match() {
        let fixture = InstallerFixture::new(false);
        let output = fixture.run("Linux", "x86_64");

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            "checksum verification failed for gh-actions-updater-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert!(!fixture.install_dir.join("gau").exists());
    }

    #[test]
    fn should_reject_unsupported_platform_before_download() {
        let fixture = InstallerFixture::new(true);
        let output = fixture.run("Plan9", "mips64");

        assert_eq!(output.status.code(), Some(2));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            "unsupported runner platform: Plan9 mips64"
        );
        assert!(!fixture.install_dir.join("gau").exists());
    }

    #[test]
    fn should_install_windows_executable_with_expected_name() {
        let fixture = WindowsInstallerFixture::new();
        let output = fixture.run();

        assert_eq!(
            output.status.code(),
            Some(0),
            "installer failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read(fixture.install_dir.join("gau.exe")).unwrap(),
            b"fixture windows executable"
        );
        assert!(!fixture.install_dir.join("gau").exists());
    }
}
