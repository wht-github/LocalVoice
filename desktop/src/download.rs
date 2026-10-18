//! Verified model downloads. Only an explicit preparation action enables network access.
use anyhow::{Context, Result, bail};
use reqwest::{StatusCode, blocking::Client, header};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    time::{Duration, Instant},
};

pub fn verify(
    path: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    let mut source = File::open(path)?;
    if source.metadata()?.len() != expected_bytes {
        bail!("模型文件不完整：{}", path.display());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        if cancelled() {
            bail!("已取消模型准备");
        }
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if format!("{:x}", hash.finalize()) != expected_sha256 {
        bail!("模型校验失败：{}", path.display());
    }
    Ok(())
}

pub fn ensure_file(
    target: &Path,
    url: &str,
    bytes: u64,
    sha256: &str,
    allow_download: bool,
    cancelled: impl Fn() -> bool,
) -> Result<()> {
    if target.is_file() {
        match verify(target, bytes, sha256, &cancelled) {
            Ok(()) => return Ok(()),
            Err(error) if !allow_download || cancelled() => return Err(error),
            Err(_) => {}
        }
    }
    if !allow_download {
        bail!("尚未安装完整整理模型，请在设置中点击“准备整理模型”");
    }
    if cancelled() {
        bail!("已取消模型准备");
    }
    fs::create_dir_all(target.parent().context("模型目录无效")?)?;
    let partial = target.with_extension("partial");
    let result = (|| -> Result<()> {
        // Bound each request so stopping preparation also bounds a stalled connection.
        // Download chunks, not the complete model, under the 30 second HTTP timeout.
        const CHUNK: u64 = 8 * 1024 * 1024;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        let started = Instant::now();
        let mut output = File::create(&partial)?;
        let mut received = 0;
        let mut buffer = [0; 64 * 1024];
        while received < bytes {
            if cancelled() {
                bail!("已取消模型准备");
            }
            if started.elapsed() > Duration::from_secs(12 * 60) {
                bail!("整理模型下载超时，请稍后重试");
            }
            let end = (received + CHUNK).min(bytes) - 1;
            let range = format!("bytes={received}-{end}");
            let expected_range = format!("bytes {received}-{end}/{bytes}");
            let mut response = client.get(url).header(header::RANGE, range).send()?;
            if response.status() != StatusCode::PARTIAL_CONTENT
                || response
                    .headers()
                    .get(header::CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    != Some(expected_range.as_str())
            {
                response.error_for_status_ref()?;
                bail!("下载服务未返回指定模型分块，请稍后重试");
            }
            let mut chunk_bytes = 0;
            loop {
                if cancelled() {
                    bail!("已取消模型准备");
                }
                let count = response.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                if received + count as u64 > end + 1 {
                    bail!("模型下载超出预期大小");
                }
                output.write_all(&buffer[..count])?;
                received += count as u64;
                chunk_bytes += count;
            }
            if received != end + 1 || chunk_bytes == 0 {
                bail!("模型下载不完整，请稍后重试");
            }
        }
        output.sync_all()?;
        drop(output);
        verify(&partial, bytes, sha256, &cancelled)?;
        if cancelled() {
            bail!("已取消模型准备");
        }
        // rename replaces only after full validation; the previous file survives failure.
        fs::rename(&partial, target).context("无法保存已验证的整理模型")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temporary() -> std::path::PathBuf {
        static ID: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "local-voice-model-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn existing_verified_model_never_connects_to_network() {
        let path = temporary();
        fs::write(&path, b"abc").unwrap();
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        ensure_file(&path, "invalid://no-network", 3, digest, true, || false).unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn cancelled_verification_and_missing_model_do_not_download() {
        let path = temporary();
        fs::write(&path, b"abc").unwrap();
        assert!(
            verify(&path, 3, "unused", &|| true)
                .unwrap_err()
                .to_string()
                .contains("取消")
        );
        fs::remove_file(&path).unwrap();
        assert!(
            ensure_file(&path, "invalid://no-network", 3, "unused", false, || false)
                .unwrap_err()
                .to_string()
                .contains("尚未安装")
        );
        assert!(!path.with_extension("partial").exists());
    }

    #[test]
    fn failed_download_verification_preserves_previous_file() {
        let path = temporary();
        fs::write(&path, b"old").unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let fixture = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = [0; 4096];
            let count = stream.read(&mut request).unwrap();
            assert!(
                String::from_utf8_lossy(&request[..count])
                    .to_lowercase()
                    .contains("range: bytes=0-2")
            );
            stream.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-2/3\r\nContent-Length: 3\r\nConnection: close\r\n\r\nnew").unwrap();
        });
        assert!(ensure_file(&path, &url, 3, "wrong-digest", true, || false).is_err());
        fixture.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert!(!path.with_extension("partial").exists());
        fs::remove_file(path).unwrap();
    }
}
