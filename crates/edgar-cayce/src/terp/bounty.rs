//! Minimal bounty board: post bounties, claim, list. Stored in-memory
//! (sqlite persistence is the natural next step).

use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct Bounty {
    pub id: u64,
    pub title: String,
    pub reward: String,
    pub status: String,
    pub poster: String,
}

pub struct BountyBoard {
    next: AtomicU64,
    items: RwLock<Vec<Bounty>>,
}

impl Default for BountyBoard {
    fn default() -> Self {
        Self {
            next: AtomicU64::new(1),
            items: RwLock::new(Vec::new()),
        }
    }
}

impl BountyBoard {
    pub async fn post(&self, title: String, reward: String, poster: String) -> Bounty {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let b = Bounty {
            id,
            title,
            reward,
            status: "open".into(),
            poster,
        };
        self.items.write().await.push(b.clone());
        b
    }
    pub async fn list(&self) -> Vec<Bounty> {
        self.items.read().await.clone()
    }
    pub async fn claim(&self, id: u64, claimer: &str) -> Option<Bounty> {
        let mut items = self.items.write().await;
        for b in items.iter_mut() {
            if b.id == id && b.status == "open" {
                b.status = format!("claimed by {}", claimer);
                return Some(b.clone());
            }
        }
        None
    }
}

pub fn format_bounty(b: &Bounty) -> String {
    format!(
        "🎯 **#{id} {title}** — {reward} (`{status}`, by {poster})",
        id = b.id,
        title = b.title,
        reward = b.reward,
        status = b.status,
        poster = b.poster
    )
}
