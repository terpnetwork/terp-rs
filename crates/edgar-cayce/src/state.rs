use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::{RwLock, Semaphore};
use tracing::info;

use crate::docs::DocumentStore;
use crate::llm::LlmClient;
use crate::rlm::RlmEngine;
use crate::terp::bounty::BountyBoard;

/// Logged until ec-gate supplies the real verdict. This stream does not call the gate.
pub const GATE_VERDICT_PENDING: &str = "pending";

const DEFAULT_COOLDOWN: Duration = Duration::from_secs(30);
const DEFAULT_MAX_CONCURRENT: usize = 1;

/// Configurable RLM parameters (admins can modify at runtime).
pub struct RlmConfig {
    pub min_code_executions: u32,
    pub max_iterations: u32,
    pub min_answer_len: usize,
    pub parallel_loops: u32,
}

impl Default for RlmConfig {
    fn default() -> Self {
        Self {
            min_code_executions: 3,
            max_iterations: 15,
            min_answer_len: 150,
            parallel_loops: 2,
        }
    }
}

pub struct AppState {
    pub store: Arc<DocumentStore>,
    pub llm: Arc<LlmClient>,
    pub rlm: Arc<RlmEngine>,
    pub admin_ids: HashSet<u64>,
    pub admin_role_ids: Arc<RwLock<HashSet<u64>>>,
    pub rlm_config: Arc<RwLock<RlmConfig>>,
    pub bounties: Arc<BountyBoard>,
}

pub type Context<'a> = poise::Context<'a, AppState, anyhow::Error>;

/// Why an ask was refused. First match wins: kill switch, pause, channel, role, cooldown, cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskDeny {
    KillSwitch,
    Paused,
    Channel,
    Role,
    Cooldown,
    Busy,
}

impl AskDeny {
    pub fn outcome(self) -> &'static str {
        match self {
            Self::KillSwitch => "denied_kill_switch",
            Self::Paused => "denied_paused",
            Self::Channel => "denied_channel",
            Self::Role => "denied_role",
            Self::Cooldown => "denied_cooldown",
            Self::Busy => "denied_busy",
        }
    }

    pub fn reply(self) -> &'static str {
        match self {
            Self::KillSwitch => "Asking is turned off.",
            Self::Paused => "Asking is paused.",
            Self::Channel => "Asking isn't enabled in this channel.",
            Self::Role => "You don't have access to ask yet.",
            Self::Cooldown => "Please wait a moment before asking again.",
            Self::Busy => {
                "I'm already working on as many questions as I can. Try again in a moment."
            }
        }
    }
}

/// Held for the duration of one admitted ask. Dropping it releases the slot.
pub struct AskLease {
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl std::fmt::Debug for AskLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AskLease")
    }
}

struct AskPilotConfig {
    enabled: bool,
    channel_ids: HashSet<u64>,
    role_ids: HashSet<u64>,
    cooldown: Duration,
    max_concurrent: usize,
}

/// Process-wide ask gate. `ASK_ENABLED` is fixed at startup. Pause and resume flip `paused` only.
pub struct AskPilot {
    enabled: AtomicBool,
    paused: AtomicBool,
    channels: HashSet<u64>,
    roles: HashSet<u64>,
    cooldown: Duration,
    slots: Arc<Semaphore>,
    last_ask: Mutex<HashMap<u64, Instant>>,
}

impl AskPilot {
    fn new(config: AskPilotConfig) -> Self {
        let permits = config.max_concurrent.max(1);
        Self {
            enabled: AtomicBool::new(config.enabled),
            paused: AtomicBool::new(false),
            channels: config.channel_ids,
            roles: config.role_ids,
            cooldown: config.cooldown,
            slots: Arc::new(Semaphore::new(permits)),
            last_ask: Mutex::new(HashMap::new()),
        }
    }

    /// Safe defaults: kill switch off, both allowlists empty, 30s cooldown, one ask at a time.
    pub fn from_values(
        enabled: Option<&str>,
        channels: &str,
        roles: &str,
        cooldown: Option<&str>,
        max_concurrent: Option<&str>,
    ) -> Self {
        Self::new(AskPilotConfig {
            enabled: flag_is_on(enabled),
            channel_ids: parse_id_list(channels),
            role_ids: parse_id_list(roles),
            cooldown: parse_cooldown(cooldown),
            max_concurrent: parse_max_concurrent(max_concurrent),
        })
    }

    pub fn from_env() -> Self {
        let enabled = std::env::var("ASK_ENABLED").ok();
        let channels = std::env::var("ASK_CHANNEL_IDS").unwrap_or_default();
        let roles = std::env::var("ASK_ROLE_IDS").unwrap_or_default();
        let cooldown = std::env::var("ASK_COOLDOWN_SECS").ok();
        let max = std::env::var("ASK_MAX_CONCURRENT").ok();
        Self::from_values(
            enabled.as_deref(),
            &channels,
            &roles,
            cooldown.as_deref(),
            max.as_deref(),
        )
    }

    pub fn get() -> &'static Self {
        static PILOT: OnceLock<AskPilot> = OnceLock::new();
        PILOT.get_or_init(Self::from_env)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
    }

    pub fn try_admit(
        &self,
        user_id: u64,
        channel_id: u64,
        role_ids: &[u64],
        now: Instant,
    ) -> Result<AskLease, AskDeny> {
        if !self.is_enabled() {
            return Err(AskDeny::KillSwitch);
        }
        if self.is_paused() {
            return Err(AskDeny::Paused);
        }
        if self.channels.is_empty() || !self.channels.contains(&channel_id) {
            return Err(AskDeny::Channel);
        }
        if self.roles.is_empty() || !role_ids.iter().any(|id| self.roles.contains(id)) {
            return Err(AskDeny::Role);
        }

        let mut last = self.last_ask.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(prev) = last.get(&user_id) {
            if now.saturating_duration_since(*prev) < self.cooldown {
                return Err(AskDeny::Cooldown);
            }
        }
        let permit = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| AskDeny::Busy)?;
        last.insert(user_id, now);
        Ok(AskLease { _permit: permit })
    }
}

pub struct AskLog<'a> {
    pub user_hash: String,
    pub channel_id: u64,
    pub gate_verdict: &'a str,
    pub profile: &'a str,
    pub latency_ms: u64,
    pub outcome: &'a str,
}

impl AskLog<'_> {
    pub fn emit(self) {
        info!(
            user_hash = %self.user_hash,
            channel = self.channel_id,
            gate_verdict = self.gate_verdict,
            profile = %self.profile,
            latency_ms = self.latency_ms,
            outcome = %self.outcome,
            "ask"
        );
    }
}

/// `gate_verdict` is `None` until ec-gate passes its verdict string.
pub fn ask_log<'a>(
    user_id: u64,
    channel_id: u64,
    profile: &'a str,
    latency_ms: u64,
    outcome: &'a str,
    gate_verdict: Option<&'a str>,
) -> AskLog<'a> {
    AskLog {
        user_hash: hash_user_id(user_id),
        channel_id,
        gate_verdict: gate_verdict.unwrap_or(GATE_VERDICT_PENDING),
        profile,
        latency_ms,
        outcome,
    }
}

fn hash_user_id(user_id: u64) -> String {
    blake3::hash(&user_id.to_le_bytes()).to_hex().to_string()
}

fn flag_is_on(raw: Option<&str>) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Empty input is an empty set. Any non-numeric entry fails the whole list closed.
fn parse_id_list(raw: &str) -> HashSet<u64> {
    let mut ids = HashSet::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.parse::<u64>() {
            Ok(id) => {
                ids.insert(id);
            }
            Err(_) => return HashSet::new(),
        }
    }
    ids
}

fn parse_cooldown(raw: Option<&str>) -> Duration {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return DEFAULT_COOLDOWN;
    };
    match raw.parse::<u64>() {
        Ok(secs) => Duration::from_secs(secs),
        Err(_) => DEFAULT_COOLDOWN,
    }
}

fn parse_max_concurrent(raw: Option<&str>) -> usize {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return DEFAULT_MAX_CONCURRENT;
    };
    match raw.parse::<usize>() {
        Ok(0) | Err(_) => DEFAULT_MAX_CONCURRENT,
        Ok(n) => n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CH: u64 = 10;
    const ROLE: u64 = 20;

    fn open_pilot(enabled: bool, max: usize, cooldown: Duration) -> AskPilot {
        AskPilot::new(AskPilotConfig {
            enabled,
            channel_ids: HashSet::from([CH]),
            role_ids: HashSet::from([ROLE]),
            cooldown,
            max_concurrent: max,
        })
    }

    fn admit(pilot: &AskPilot, user: u64, now: Instant) -> Result<AskLease, AskDeny> {
        pilot.try_admit(user, CH, &[ROLE], now)
    }

    #[test]
    fn allowlist_denies_when_unset() {
        let now = Instant::now();
        let unset = AskPilot::from_values(Some("true"), "", "", Some("0"), Some("1"));
        assert_eq!(
            unset.try_admit(1, CH, &[ROLE], now).unwrap_err(),
            AskDeny::Channel
        );

        let no_roles = AskPilot::from_values(Some("true"), "10, 11", "", Some("0"), Some("1"));
        assert_eq!(
            no_roles.try_admit(1, CH, &[ROLE], now).unwrap_err(),
            AskDeny::Role
        );

        let pilot = AskPilot::from_values(Some("true"), "10, 11", "20, 21", Some("0"), Some("2"));
        assert_eq!(
            pilot.try_admit(1, 99, &[ROLE], now).unwrap_err(),
            AskDeny::Channel
        );
        assert_eq!(
            pilot.try_admit(1, CH, &[99], now).unwrap_err(),
            AskDeny::Role
        );
        assert_eq!(pilot.try_admit(1, CH, &[], now).unwrap_err(), AskDeny::Role);
        assert!(pilot.try_admit(1, 11, &[7, ROLE], now).is_ok());

        let bad = AskPilot::from_values(Some("true"), "10, nope", "20", Some("0"), Some("1"));
        assert_eq!(
            bad.try_admit(1, CH, &[ROLE], now).unwrap_err(),
            AskDeny::Channel
        );
    }

    #[test]
    fn kill_switch_defaults_off() {
        assert!(!flag_is_on(None));
        assert!(!flag_is_on(Some("")));
        assert!(!flag_is_on(Some("false")));
        assert!(!flag_is_on(Some("0")));
        assert!(!flag_is_on(Some("no")));
        assert!(flag_is_on(Some("true")));
        assert!(flag_is_on(Some("TRUE")));
        assert!(flag_is_on(Some("1")));
        assert!(flag_is_on(Some("yes")));
        assert!(flag_is_on(Some("on")));

        let now = Instant::now();
        let pilot = AskPilot::from_values(None, "10", "20", Some("0"), Some("1"));
        let err = pilot.try_admit(1, CH, &[ROLE], now).unwrap_err();
        assert_eq!(err, AskDeny::KillSwitch);
        assert_eq!(err.outcome(), "denied_kill_switch");

        let pilot = open_pilot(false, 1, Duration::ZERO);
        pilot.resume();
        assert_eq!(admit(&pilot, 1, now).unwrap_err(), AskDeny::KillSwitch);
    }

    #[test]
    fn pause_and_resume_toggle_runtime_gate() {
        let pilot = open_pilot(true, 2, Duration::ZERO);
        let now = Instant::now();
        let held = admit(&pilot, 1, now).unwrap();
        pilot.pause();
        assert!(pilot.is_paused());
        assert_eq!(admit(&pilot, 2, now).unwrap_err(), AskDeny::Paused);
        pilot.pause();
        assert_eq!(admit(&pilot, 2, now).unwrap_err(), AskDeny::Paused);
        drop(held);
        pilot.resume();
        assert!(!pilot.is_paused());
        assert!(admit(&pilot, 2, now).is_ok());
    }

    #[test]
    fn cooldown_is_per_user() {
        let pilot = open_pilot(true, 4, Duration::from_secs(30));
        let t0 = Instant::now();
        let first = admit(&pilot, 1, t0).unwrap();
        assert_eq!(
            admit(&pilot, 1, t0 + Duration::from_secs(29)).unwrap_err(),
            AskDeny::Cooldown
        );
        assert!(admit(&pilot, 2, t0 + Duration::from_secs(1)).is_ok());
        drop(first);
        assert_eq!(
            admit(&pilot, 1, t0 + Duration::from_secs(29)).unwrap_err(),
            AskDeny::Cooldown
        );
        assert!(admit(&pilot, 1, t0 + Duration::from_secs(30)).is_ok());

        // Unset ASK_COOLDOWN_SECS is 30.
        let pilot = AskPilot::from_values(Some("true"), "10", "20", None, Some("4"));
        let held = admit(&pilot, 7, t0).unwrap();
        assert_eq!(
            admit(&pilot, 7, t0 + Duration::from_secs(29)).unwrap_err(),
            AskDeny::Cooldown
        );
        drop(held);
        assert!(admit(&pilot, 7, t0 + Duration::from_secs(30)).is_ok());
    }

    #[test]
    fn concurrency_cap_rejects_when_full() {
        let pilot = open_pilot(true, 1, Duration::ZERO);
        let now = Instant::now();
        let held = admit(&pilot, 1, now).unwrap();
        assert_eq!(admit(&pilot, 2, now).unwrap_err(), AskDeny::Busy);
        drop(held);
        assert!(admit(&pilot, 2, now).is_ok());

        let pilot = AskPilot::from_values(Some("true"), "10", "20", Some("0"), None);
        let held = admit(&pilot, 1, now).unwrap();
        assert_eq!(admit(&pilot, 2, now).unwrap_err(), AskDeny::Busy);
        drop(held);

        let pilot = open_pilot(true, 2, Duration::ZERO);
        let a = admit(&pilot, 1, now).unwrap();
        let b = admit(&pilot, 2, now).unwrap();
        assert_eq!(admit(&pilot, 3, now).unwrap_err(), AskDeny::Busy);
        drop(a);
        assert!(admit(&pilot, 3, now).is_ok());
        drop(b);
    }

    #[test]
    fn ask_log_hashes_the_user_and_leaves_gate_pending() {
        let user_id = 123456789012345678u64;
        let log = ask_log(
            user_id,
            CH,
            "opencode/deepseek-v4.1-flash-free",
            15,
            "ok",
            None,
        );
        assert_eq!(log.user_hash.len(), 64);
        assert!(log.user_hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(log.user_hash, user_id.to_string());
        assert!(!log.user_hash.contains(&user_id.to_string()));
        assert_eq!(hash_user_id(user_id), log.user_hash);
        assert_ne!(hash_user_id(user_id), hash_user_id(user_id + 1));
        assert_eq!(log.channel_id, CH);
        assert_eq!(log.gate_verdict, "pending");
        assert_eq!(log.gate_verdict, GATE_VERDICT_PENDING);
        let held = ask_log(user_id, CH, "none", 0, "ok", Some("hold"));
        assert_eq!(held.gate_verdict, "hold");
        assert_eq!(log.profile, "opencode/deepseek-v4.1-flash-free");
        assert_eq!(log.latency_ms, 15);
        assert_eq!(log.outcome, "ok");
    }
}
