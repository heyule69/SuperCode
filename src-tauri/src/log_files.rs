//! UTF-8 JSONL journal: append-only segments, 30-day retention, streaming exports.
use chrono::{Days, NaiveDate, Utc};
use serde_json::Value;
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub const RETENTION_DAYS: u64 = 30;
const SEGMENT_BYTES: u64 = 4 * 1024 * 1024;
const LINE_BYTES: usize = 64 * 1024;
const CLEANUP_SECONDS: i64 = 3600;

struct Active {
    date: NaiveDate,
    index: u32,
    file: File,
    bytes: u64,
}
pub struct Journal {
    pub directory: PathBuf,
    active: Option<Active>,
    last_cleanup: i64,
}
pub struct Snapshot(pub Vec<(File, u64)>);

fn filename(date: NaiveDate, index: u32) -> String {
    format!("supercode-{date}-{index:04}.jsonl")
}
fn owned_date(name: &str) -> Option<NaiveDate> {
    let stem = name.strip_prefix("supercode-")?.strip_suffix(".jsonl")?;
    let date = stem.get(..10)?;
    let segment = stem.get(10..)?.strip_prefix('-')?;
    if !(segment == "panic"
        || segment
            .strip_prefix("panic-")
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
        || (segment.len() == 4 && segment.bytes().all(|b| b.is_ascii_digit())))
    {
        return None;
    }
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}
fn regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}
fn exists_without_following_links(path: &Path) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}
fn validate(path: &Path) -> Result<(), String> {
    if !regular_file(path) {
        return Err("日志文件不是普通文件，未修改原文件".into());
    }
    let mut reader = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    let mut line = Vec::new();
    loop {
        line.clear();
        let bytes = reader
            .by_ref()
            .take((LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        if bytes == 0 {
            break;
        }
        if bytes > LINE_BYTES {
            return Err("日志记录超过大小限制，未修改原文件".into());
        }
        let text = std::str::from_utf8(&line).map_err(|_| "日志不是 UTF-8 编码，未修改原文件")?;
        if text.starts_with('\u{feff}') {
            return Err("日志含 UTF-8 BOM，未修改原文件".into());
        }
        serde_json::from_str::<Value>(text).map_err(|_| "日志记录不完整，未修改原文件")?;
    }
    Ok(())
}

impl Journal {
    pub fn new(data: &Path, now: i64) -> Result<Self, String> {
        let root = data.canonicalize().map_err(|e| e.to_string())?;
        let directory = root.join("logs");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let directory = directory.canonicalize().map_err(|e| e.to_string())?;
        if !directory.starts_with(&root) {
            return Err("日志目录位于应用数据目录之外，未清理任何文件".into());
        }
        let mut journal = Self {
            directory,
            active: None,
            last_cleanup: 0,
        };
        if let Err(error) = journal.cleanup(now) {
            eprintln!("日志清理失败，将继续保存新日志：{error}");
        }
        Ok(journal)
    }
    pub fn cleanup(&mut self, now: i64) -> Result<(), String> {
        let today = chrono::DateTime::from_timestamp(now, 0)
            .ok_or("日志时间无效")?
            .date_naive();
        let cutoff = today
            .checked_sub_days(Days::new(RETENTION_DAYS))
            .ok_or("日志保留时间无效")?;
        self.last_cleanup = now;
        for entry in std::fs::read_dir(&self.directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            // Never recurse or follow links. Only application-owned, expired JSONL files qualify.
            if entry.file_type().map_err(|e| e.to_string())?.is_file()
                && entry
                    .file_name()
                    .to_str()
                    .and_then(owned_date)
                    .is_some_and(|date| date < cutoff)
            {
                std::fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
        self.last_cleanup = now;
        Ok(())
    }
    pub fn cleanup_due(&mut self, now: i64) -> Result<(), String> {
        if now.saturating_sub(self.last_cleanup) >= CLEANUP_SECONDS {
            self.cleanup(now)?;
        }
        Ok(())
    }
    fn select(&mut self, date: NaiveDate, length: u64) -> Result<Vec<String>, String> {
        let mut warnings = Vec::new();
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.date == date && a.bytes + length <= SEGMENT_BYTES)
        {
            return Ok(warnings);
        }
        let mut index = self
            .active
            .as_ref()
            .filter(|a| a.date == date)
            .map_or(0, |a| a.index + 1);
        for entry in std::fs::read_dir(&self.directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if let Some(name) = entry.file_name().to_str() {
                if owned_date(name) == Some(date) {
                    if let Some(number) = name
                        .strip_suffix(".jsonl")
                        .and_then(|s| s.rsplit('-').next())
                        .filter(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit()))
                        .and_then(|s| s.parse::<u32>().ok())
                    {
                        index = index.max(number);
                    }
                }
            }
        }
        loop {
            if index > 9999 {
                return Err("当日日志分段数量超过限制".into());
            }
            let path = self.directory.join(filename(date, index));
            let exists = exists_without_following_links(&path)?;
            let bytes = if exists {
                if let Err(error) = validate(&path) {
                    warnings.push(format!("{}：{error}；已另建日志文件", path.display()));
                    index += 1;
                    continue;
                }
                std::fs::metadata(&path).map_err(|e| e.to_string())?.len()
            } else {
                0
            };
            if bytes + length > SEGMENT_BYTES {
                index += 1;
                continue;
            }
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .read(true)
                .open(&path)
                .map_err(|e| e.to_string())?;
            if bytes > 0 {
                file.seek(SeekFrom::End(-1)).map_err(|e| e.to_string())?;
                let mut last = [0];
                file.read_exact(&mut last).map_err(|e| e.to_string())?;
                if last[0] != b'\n' {
                    file.write_all(b"\n").map_err(|e| e.to_string())?;
                }
            }
            let actual = file.metadata().map_err(|e| e.to_string())?.len();
            self.active = Some(Active {
                date,
                index,
                file,
                bytes: actual,
            });
            return Ok(warnings);
        }
    }
    pub fn append(
        &mut self,
        now: i64,
        value: &Value,
        durable: bool,
    ) -> Result<Vec<String>, String> {
        let mut warnings: Vec<String> = self
            .cleanup_due(now)
            .err()
            .into_iter()
            .map(|e| format!("日志清理失败：{e}"))
            .collect();
        let date = chrono::DateTime::from_timestamp(now, 0)
            .ok_or("日志时间无效")?
            .date_naive();
        let line = format!("{value}\n");
        if line.len() > LINE_BYTES {
            return Err("单条日志超过大小限制".into());
        }
        warnings.extend(self.select(date, line.len() as u64)?);
        let active = self.active.as_mut().ok_or("日志文件未打开")?;
        active
            .file
            .write_all(line.as_bytes())
            .map_err(|e| e.to_string())?;
        active.bytes += line.len() as u64;
        if durable {
            active.file.sync_data().map_err(|e| e.to_string())?;
        }
        Ok(warnings)
    }
    pub fn snapshot(&mut self, now: i64) -> Result<Snapshot, String> {
        if let Err(error) = self.cleanup(now) {
            eprintln!("日志清理失败，将继续导出保留期内的日志：{error}");
        }
        let cutoff = chrono::DateTime::from_timestamp(now, 0)
            .ok_or("日志时间无效")?
            .date_naive()
            .checked_sub_days(Days::new(RETENTION_DAYS))
            .ok_or("日志保留时间无效")?;
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(&self.directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_file()
                && entry
                    .file_name()
                    .to_str()
                    .and_then(owned_date)
                    .is_some_and(|date| date >= cutoff)
            {
                paths.push(entry.path());
            }
            if paths.len() > 10000 {
                return Err("日志文件过多，请从日志文件夹分批收集".into());
            }
        }
        paths.sort();
        let mut files = Vec::new();
        for path in paths {
            let file = File::open(&path).map_err(|e| e.to_string())?;
            let length = file.metadata().map_err(|e| e.to_string())?.len();
            files.push((file, length));
        }
        Ok(Snapshot(files))
    }
}

impl Snapshot {
    pub fn export(self, destination: &Path, header: &Value) -> Result<(), String> {
        let parent = destination.parent().ok_or("导出位置无效")?;
        let temporary = parent.join(format!(
            ".supercode-log-export-{}.tmp",
            uuid::Uuid::new_v4()
        ));
        let result = (|| {
            let file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|e| e.to_string())?;
            let mut output = BufWriter::new(file);
            writeln!(output, "{header}").map_err(|e| e.to_string())?;
            for (file, length) in self.0 {
                let mut reader = BufReader::new(file.take(length));
                let mut line = Vec::new();
                loop {
                    line.clear();
                    let count = reader
                        .by_ref()
                        .take((LINE_BYTES + 1) as u64)
                        .read_until(b'\n', &mut line)
                        .map_err(|e| e.to_string())?;
                    if count == 0 {
                        break;
                    }
                    if count > LINE_BYTES {
                        return Err("日志记录超过大小限制，导出文件未替换".into());
                    }
                    let text = std::str::from_utf8(&line)
                        .map_err(|_| "日志不是 UTF-8 编码，导出文件未替换")?;
                    if text.starts_with('\u{feff}') {
                        return Err("日志含 UTF-8 BOM，导出文件未替换".into());
                    }
                    serde_json::from_str::<Value>(text)
                        .map_err(|_| "日志记录不完整，导出文件未替换")?;
                    output.write_all(&line).map_err(|e| e.to_string())?;
                    if !line.ends_with(b"\n") {
                        output.write_all(b"\n").map_err(|e| e.to_string())?;
                    }
                }
            }
            output.flush().map_err(|e| e.to_string())?;
            output.get_ref().sync_all().map_err(|e| e.to_string())?;
            drop(output);
            std::fs::rename(&temporary, destination).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

pub fn panic_record(directory: &Path, value: &Value) -> Result<(), String> {
    let mut path = directory.join(format!("supercode-{}-panic.jsonl", Utc::now().date_naive()));
    if exists_without_following_links(&path)? {
        if let Err(error) = validate(&path) {
            eprintln!("{}：{error}；已另建崩溃日志", path.display());
            path = directory.join(format!(
                "supercode-{}-panic-{}.jsonl",
                Utc::now().date_naive(),
                uuid::Uuid::new_v4()
            ));
        }
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 0 {
        file.seek(SeekFrom::End(-1)).map_err(|e| e.to_string())?;
        let mut last = [0];
        file.read_exact(&mut last).map_err(|e| e.to_string())?;
        if last[0] != b'\n' {
            file.write_all(b"\n").map_err(|e| e.to_string())?;
        }
    }
    file.write_all(format!("{value}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    file.sync_data().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("supercode-log-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let (Ok(path), Ok(root)) =
                (self.0.canonicalize(), std::env::temp_dir().canonicalize())
            {
                if path.parent() == Some(root.as_path())
                    && path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("supercode-log-test-"))
                {
                    let _ = std::fs::remove_dir_all(path);
                }
            }
        }
    }
    fn time(day: &str) -> i64 {
        NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp()
    }
    #[test]
    fn reopening_and_date_rotation_preserve_records_in_utf8() {
        let fixture = Fixture::new();
        {
            let mut journal = Journal::new(&fixture.0, time("2026-10-10")).unwrap();
            journal
                .append(time("2026-10-10"), &json!({"event":"异常一"}), true)
                .unwrap();
        }
        let mut journal = Journal::new(&fixture.0, time("2026-10-11")).unwrap();
        journal
            .append(time("2026-10-11"), &json!({"event":"异常二"}), true)
            .unwrap();
        let output = fixture.0.join("export.jsonl");
        journal
            .snapshot(time("2026-10-11"))
            .unwrap()
            .export(&output, &json!({"export":true}))
            .unwrap();
        let text = std::fs::read_to_string(output).unwrap();
        assert!(text.contains("异常一") && text.contains("异常二"));
        assert!(!text.starts_with('\u{feff}'));
    }
    #[test]
    fn simultaneous_sessions_keep_complete_records_without_overwriting_each_other() {
        let fixture = Fixture::new();
        let now = time("2026-10-11");
        let journal = std::sync::Arc::new(std::sync::Mutex::new(
            Journal::new(&fixture.0, now).unwrap(),
        ));
        let threads = (0..4)
            .map(|session| {
                let journal = journal.clone();
                std::thread::spawn(move || {
                    for sequence in 0..25 {
                        journal
                            .lock()
                            .unwrap()
                            .append(
                                now,
                                &json!({"sessionId":session,"sequence":sequence}),
                                false,
                            )
                            .unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().unwrap();
        }
        let output = fixture.0.join("concurrent.jsonl");
        journal
            .lock()
            .unwrap()
            .snapshot(now)
            .unwrap()
            .export(&output, &json!({"export":true}))
            .unwrap();
        let records = std::fs::read_to_string(output)
            .unwrap()
            .lines()
            .skip(1)
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 100);
        for session in 0..4 {
            assert_eq!(
                records.iter().filter(|r| r["sessionId"] == session).count(),
                25
            );
        }
    }
    #[test]
    fn size_rotation_keeps_old_segments_instead_of_discarding_recent_errors() {
        let fixture = Fixture::new();
        let now = time("2026-10-11");
        let mut journal = Journal::new(&fixture.0, now).unwrap();
        journal
            .append(now, &json!({"event":"first"}), false)
            .unwrap();
        journal.active.as_mut().unwrap().bytes = SEGMENT_BYTES;
        journal
            .append(now, &json!({"event":"second"}), false)
            .unwrap();
        assert_eq!(journal.active.as_ref().unwrap().index, 1);
        assert!(std::fs::read_to_string(
            journal
                .directory
                .join(filename(NaiveDate::from_ymd_opt(2026, 10, 11).unwrap(), 0))
        )
        .unwrap()
        .contains("first"));
    }
    #[test]
    fn panic_uuid_suffixes_are_not_mistaken_for_segment_numbers() {
        let fixture = Fixture::new();
        let now = time("2026-10-11");
        let mut journal = Journal::new(&fixture.0, now).unwrap();
        std::fs::write(
            journal
                .directory
                .join("supercode-2026-10-11-panic-00000000-0000-0000-0000-000000020000.jsonl"),
            "{}\n",
        )
        .unwrap();
        journal
            .append(now, &json!({"event":"after_panic"}), true)
            .unwrap();
        assert_eq!(journal.active.as_ref().unwrap().index, 0);
    }
    #[test]
    fn panic_records_remain_separate_when_an_existing_record_has_no_newline() {
        let fixture = Fixture::new();
        let journal = Journal::new(&fixture.0, Utc::now().timestamp()).unwrap();
        let path = journal
            .directory
            .join(format!("supercode-{}-panic.jsonl", Utc::now().date_naive()));
        std::fs::write(&path, "{\"event\":\"previous_panic\"}").unwrap();
        panic_record(&journal.directory, &json!({"event":"next_panic"})).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let records = text
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1]["event"], "next_panic");
    }
    #[test]
    fn cleanup_deletes_only_owned_files_older_than_thirty_days_and_runs_hourly() {
        let fixture = Fixture::new();
        let now = time("2026-10-11");
        let mut journal = Journal::new(&fixture.0, now).unwrap();
        for name in [
            "supercode-2026-09-10-0000.jsonl",
            "supercode-2026-09-11-0000.jsonl",
            "supercode-2026-09-10-panic.jsonl",
            "supercode-2026-09-10-panic-00000000-0000-0000-0000-000000000000.jsonl",
            "notes.jsonl",
            "supercode-2026-99-99-0000.jsonl",
        ] {
            std::fs::write(journal.directory.join(name), "{}\n").unwrap();
        }
        std::fs::create_dir(journal.directory.join("supercode-2026-09-10-0001.jsonl")).unwrap();
        journal.cleanup_due(now + 3599).unwrap();
        assert!(journal
            .directory
            .join("supercode-2026-09-10-0000.jsonl")
            .exists());
        journal.cleanup_due(now + 3600).unwrap();
        assert!(!journal
            .directory
            .join("supercode-2026-09-10-0000.jsonl")
            .exists());
        assert!(!journal
            .directory
            .join("supercode-2026-09-10-panic.jsonl")
            .exists());
        assert!(!journal
            .directory
            .join("supercode-2026-09-10-panic-00000000-0000-0000-0000-000000000000.jsonl")
            .exists());
        for name in [
            "supercode-2026-09-11-0000.jsonl",
            "notes.jsonl",
            "supercode-2026-99-99-0000.jsonl",
            "supercode-2026-09-10-0001.jsonl",
        ] {
            assert!(journal.directory.join(name).exists());
        }
    }
    #[test]
    fn invalid_utf8_is_reported_preserved_and_logging_continues_in_a_new_segment() {
        let fixture = Fixture::new();
        let now = time("2026-10-11");
        let mut journal = Journal::new(&fixture.0, now).unwrap();
        let original = journal
            .directory
            .join(filename(NaiveDate::from_ymd_opt(2026, 10, 11).unwrap(), 0));
        std::fs::write(&original, [0xff, 0xfe]).unwrap();
        let warnings = journal
            .append(now, &json!({"event":"still_logged"}), true)
            .unwrap();
        assert!(warnings[0].contains("UTF-8"));
        assert_eq!(std::fs::read(original).unwrap(), [0xff, 0xfe]);
        assert_eq!(journal.active.as_ref().unwrap().index, 1);
    }
    #[test]
    fn export_has_a_stable_snapshot_and_preserves_existing_destination_on_error() {
        let fixture = Fixture::new();
        let now = time("2026-10-11");
        let mut journal = Journal::new(&fixture.0, now).unwrap();
        journal
            .append(now, &json!({"event":"before"}), true)
            .unwrap();
        let snapshot = journal.snapshot(now).unwrap();
        journal
            .append(now, &json!({"event":"after"}), true)
            .unwrap();
        let output = fixture.0.join("export.txt");
        snapshot.export(&output, &json!({"export":true})).unwrap();
        let text = std::fs::read_to_string(&output).unwrap();
        assert!(text.contains("before") && !text.contains("after"));
        std::fs::write(
            journal
                .directory
                .join(filename(NaiveDate::from_ymd_opt(2026, 10, 11).unwrap(), 1)),
            [0xff],
        )
        .unwrap();
        assert!(journal
            .snapshot(now)
            .unwrap()
            .export(&output, &json!({"export":true}))
            .is_err());
        assert_eq!(std::fs::read_to_string(output).unwrap(), text);
    }
    #[test]
    fn panic_logging_preserves_invalid_utf8_and_writes_a_separate_retained_file() {
        let fixture = Fixture::new();
        let now = Utc::now().timestamp();
        let journal = Journal::new(&fixture.0, now).unwrap();
        let original = journal
            .directory
            .join(format!("supercode-{}-panic.jsonl", Utc::now().date_naive()));
        std::fs::write(&original, [0xff]).unwrap();
        panic_record(
            &journal.directory,
            &json!({"event":"rust_panic","message":"崩溃原因"}),
        )
        .unwrap();
        assert_eq!(std::fs::read(original).unwrap(), [0xff]);
        let files = std::fs::read_dir(&journal.directory)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(files.len(), 2);
        assert!(files
            .iter()
            .all(|f| owned_date(f.file_name().to_str().unwrap()).is_some()));
        assert!(files
            .iter()
            .any(|f| std::fs::read_to_string(f.path()).is_ok_and(|s| s.contains("崩溃原因"))));
    }
    #[cfg(windows)]
    #[test]
    fn a_locked_expired_file_does_not_prevent_recording_new_errors() {
        use std::os::windows::fs::OpenOptionsExt;
        let fixture = Fixture::new();
        let now = time("2026-10-11");
        let mut journal = Journal::new(&fixture.0, now).unwrap();
        let expired = journal.directory.join("supercode-2026-09-10-0000.jsonl");
        std::fs::write(&expired, "{}\n").unwrap();
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&expired)
            .unwrap();
        journal.last_cleanup = now - 3600;
        let warnings = journal
            .append(now, &json!({"event":"new_error"}), true)
            .unwrap();
        assert!(!warnings.is_empty());
        assert!(journal.active.as_ref().unwrap().bytes > 0);
        drop(locked);
        journal.cleanup(now).unwrap();
        assert!(!expired.exists());
    }
}
