use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerPhase {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
    Buffering,
    Stopping,
    Error,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PlayerState {
    pub session_id: u64,
    pub phase: PlayerPhase,
    pub loaded: bool,
    pub playing: bool,
    pub buffering: bool,
    pub position: f64,
    pub duration: f64,
    pub speed: f64,
    pub volume: i64,
    pub muted: bool,
    pub fullscreen: bool,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub number: i32,
    pub error: Option<String>,
}
