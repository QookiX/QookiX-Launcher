//! Wire payloads shared by task producers.
use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress<'a> {
    pub task_id: u64,
    pub stage: &'a str,
    pub message: &'a str,
    pub done: usize,
    pub total: usize,
    pub instance_id: &'a str,
    pub instance_name: &'a str,
    pub source: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancellable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveFile<'a> {
    pub name: &'a str,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress<'a> {
    pub task_id: u64,
    pub phase: &'a str,
    pub done: u64,
    pub total: usize,
    pub current: &'a str,
    pub ok: bool,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub verified_bytes: u64,
    pub ts: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_files: Option<Vec<ActiveFile<'a>>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchProgress<'a> {
    pub instance_id: &'a str,
    pub task_id: u64,
    pub step: &'a str,
    pub progress: u8,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LaunchState {
    Running,
    Exited,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchStateEvent<'a> {
    pub instance_id: &'a str,
    pub task_id: u64,
    pub state: LaunchState,
    pub pid: u32,
    pub code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_event_examples_match_frontend_wire_contract() {
        let install = InstallProgress {
            task_id: 7,
            stage: "done",
            message: "failed",
            done: 0,
            total: 0,
            instance_id: "abc123",
            instance_name: "Example",
            source: "install",
            cancellable: None,
            ok: Some(false),
        };
        let download = DownloadProgress {
            task_id: 7,
            phase: "assets",
            done: 0,
            total: 1,
            current: "",
            ok: true,
            bytes_done: 5,
            bytes_total: 9,
            verified_bytes: 0,
            ts: 100,
            active_files: Some(vec![ActiveFile {
                name: "example",
                bytes_done: 5,
                bytes_total: 9,
            }]),
        };
        let launch = LaunchStateEvent {
            instance_id: "abc123",
            task_id: 8,
            state: LaunchState::Running,
            pid: 123,
            code: None,
        };
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../src/api/wire-contracts.json")).unwrap();
        assert_eq!(serde_json::to_value(install).unwrap(), fixtures["install"]);
        assert_eq!(
            serde_json::to_value(download).unwrap(),
            fixtures["download"]
        );
        assert_eq!(serde_json::to_value(launch).unwrap(), fixtures["launch"]);
    }
}
