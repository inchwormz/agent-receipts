use crate::schema::DecisionLogRecord;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JournalEvent<T> {
    pub run_id: String,
    pub pass_id: String,
    pub event_kind: String,
    pub payload: T,
}

pub fn append_event_jsonl<T>(path: &Path, event: &JournalEvent<T>) -> io::Result<()>
where
    T: Serialize,
{
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, event).map_err(io::Error::other)?;
    file.write_all(b"\n")?;
    Ok(())
}

pub fn append_decision_log(path: &Path, record: &DecisionLogRecord) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, record).map_err(io::Error::other)?;
    file.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{JournalEvent, append_decision_log, append_event_jsonl};
    use crate::schema::DecisionLogRecord;
    use serde_json::Value;
    use std::fs;

    #[test]
    fn appends_jsonl_events() {
        let path = std::env::temp_dir().join("receipts-compiler-journal.jsonl");
        let _ = fs::remove_file(&path);
        let event = JournalEvent {
            run_id: "run-1".to_string(),
            pass_id: "pass-1".to_string(),
            event_kind: "packet-built".to_string(),
            payload: serde_json::json!({"ok": true}),
        };

        append_event_jsonl(&path, &event).expect("append event");

        let written = fs::read_to_string(&path).expect("read journal");
        let first_line = written.lines().next().expect("line");
        let parsed: Value = serde_json::from_str(first_line).expect("valid json");
        assert_eq!(parsed["event_kind"], "packet-built");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn appends_raw_decision_log_records() {
        let path = std::env::temp_dir().join("receipts-compiler-decisions.jsonl");
        let _ = fs::remove_file(&path);

        let record = DecisionLogRecord {
            id: "decision-1".to_string(),
            run_id: "run-1".to_string(),
            pass_id: "pass-1".to_string(),
            decision_kind: "promote".to_string(),
            summary: "promote directive".to_string(),
            source_ids: vec!["src-1".to_string()],
            selected_action_ids: vec!["act-1".to_string()],
            created_at: "2026-04-21T00:00:00Z".to_string(),
            promotion: None,
        };

        append_decision_log(&path, &record).expect("append decision log");

        let written = fs::read_to_string(&path).expect("read decisions");
        let first_line = written.lines().next().expect("line");
        let parsed: Value = serde_json::from_str(first_line).expect("valid json");
        assert_eq!(parsed["id"], "decision-1");
        assert!(parsed.get("payload").is_none());

        let _ = fs::remove_file(&path);
    }
}
