//! Locate the clone from the executable, independent of the launch directory.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

fn find_root(executable: &Path) -> Option<PathBuf> {
    executable
        .ancestors()
        .skip(1)
        .find(|directory| {
            directory.join("asr_server.py").is_file() && directory.join("desktop.ps1").is_file()
        })
        .map(Path::to_path_buf)
}

pub fn root() -> Result<PathBuf> {
    find_root(&std::env::current_exe()?)
        .context("未找到项目仓库；请保留程序在 .runtime/desktop 中，并从 desktop.ps1 启动")
}

pub fn runtime() -> Result<PathBuf> {
    Ok(root()?.join(".runtime"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
