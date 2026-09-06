//! Bounded timing-only diagnostics: never store audio, recognized text or target names.
use std::{
    io::Write,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
static LOCK: Mutex<()> = Mutex::new(());

pub fn record(event: &str, fields: serde_json::Value) {
    let Ok(_guard) = LOCK.lock() else { return };
    let Some(directory) = crate::config::Config::path().parent().map(|p| p.to_owned()) else {
        return;
    };
    let _ = std::fs::create_dir_all(&directory);
    let path = directory.join("timings.jsonl");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 512 * 1024) {
        let _ = std::fs::write(&path, []);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let row = serde_json::json!({"time_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),"event":event,"metrics":fields});
        let _ = writeln!(file, "{row}");
    }
}
