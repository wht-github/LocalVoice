//! Locate a portable installation or source checkout, independent of the launch directory.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

fn find_root(executable: &Path) -> Option<PathBuf> {
    executable
        .ancestors()
        .skip(1)
        .find(|directory| {
            is_portable(directory)
                || (directory.join("desktop/Cargo.toml").is_file()
                    && directory.join("desktop.ps1").is_file())
        })
        .map(Path::to_path_buf)
}

pub fn root() -> Result<PathBuf> {
    find_root(&std::env::current_exe()?).context(
        "未找到 LocalVoice 安装目录；请完整解压下载的 ZIP，保留程序旁的 localvoice.portable 文件",
    )
}

pub fn is_portable(directory: &Path) -> bool {
    std::fs::read_to_string(directory.join("localvoice.portable"))
        .is_ok_and(|contents| contents.trim_end_matches(['\r', '\n']) == "LocalVoice portable v1")
}

pub fn runtime() -> Result<PathBuf> {
    Ok(root()?.join(".runtime"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "LocalVoice 安装测试 {} {}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn write(&self, relative: &str, content: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn installed_and_development_executables_find_the_same_clone() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        for relative in [
            ".runtime/desktop/local-voice-desktop.exe",
            "desktop/target/release/local-voice-desktop.exe",
            "desktop/target/debug/deps/local-voice-desktop-test.exe",
        ] {
            assert_eq!(find_root(&root.join(relative)).as_deref(), Some(root));
        }
        assert!(find_root(Path::new("C:/not-a-localvoice-clone/app.exe")).is_none());
    }

    #[test]
    fn standalone_portable_installation_needs_no_checkout() {
        let directory = Directory::new();
        directory.write("localvoice.portable", "LocalVoice portable v1\r\n");
        let executable = directory.0.join("local-voice-desktop.exe");
        assert_eq!(find_root(&executable), Some(directory.0.clone()));
        assert!(!directory.0.join("desktop/Cargo.toml").exists());
    }

    #[test]
    fn nested_portable_installation_uses_its_own_models() {
        let directory = Directory::new();
        directory.write("desktop/Cargo.toml", "");
        directory.write("desktop.ps1", "");
        directory.write("unpacked/localvoice.portable", "LocalVoice portable v1");
        assert_eq!(
            find_root(&directory.0.join("unpacked/local-voice-desktop.exe")),
            Some(directory.0.join("unpacked"))
        );
        assert_eq!(
            find_root(
                &directory
                    .0
                    .join("desktop/target/release/local-voice-desktop.exe")
            ),
            Some(directory.0.clone())
        );
    }

    #[test]
    fn unrelated_metadata_or_invalid_marker_does_not_define_an_installation() {
        let directory = Directory::new();
        directory.write("build-info.json", "{}");
        directory.write("localvoice.portable", "another app");
        assert!(find_root(&directory.0.join("app.exe")).is_none());
    }
}
