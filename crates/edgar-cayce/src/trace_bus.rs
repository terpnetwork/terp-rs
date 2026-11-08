//! Fan-in for live Discord traces: RLM steps + selected tracing events
//! (INFO/WARN from `discord_demo::*` only — not wasmer/hyper DEBUG).

use std::sync::{Mutex, OnceLock};

use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use crate::rlm::TraceTx;

static SINK: OnceLock<Mutex<Option<TraceTx>>> = OnceLock::new();

fn sink() -> &'static Mutex<Option<TraceTx>> {
    SINK.get_or_init(|| Mutex::new(None))
}

pub fn attach(tx: TraceTx) {
    *sink().lock().unwrap() = Some(tx);
}

pub fn detach() {
    *sink().lock().unwrap() = None;
}

pub fn push(line: impl Into<String>) {
    if let Ok(g) = sink().lock() {
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(line.into());
        }
    }
}

/// Strip HTML/markdown noise so traces stay one-line-ish.
pub fn scrub(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => {
                if c.is_whitespace() {
                    if !out.ends_with(' ') && !out.is_empty() {
                        out.push(' ');
                    }
                } else {
                    out.push(c);
                }
            }
            _ => {}
        }
    }
    let out = out.replace("[Skip to content]", "");
    let t = crate::util::truncate(out.trim(), max);
    if t.len() < out.trim().len() {
        format!("{t}…")
    } else {
        t.to_string()
    }
}

pub struct DiscordTraceLayer;

impl<S> Layer<S> for DiscordTraceLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let target = meta.target();
        if !target.starts_with("discord_demo::") {
            return;
        }
        // Skip ultra-noisy modules even at INFO.
        if target.contains("wasmer") {
            return;
        }
        let level = *meta.level();
        if level > tracing::Level::INFO {
            return;
        }
        let mut vis = MessageVisitor {
            message: String::new(),
        };
        event.record(&mut vis);
        if vis.message.is_empty() {
            return;
        }
        let tag = target.rsplit("::").next().unwrap_or(target);
        push(format!("`{level}` {tag}: {}", scrub(&vis.message, 280)));
    }
}

struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
            if self.message.starts_with('"') && self.message.ends_with('"') && self.message.len() >= 2
            {
                self.message = self.message[1..self.message.len() - 1].to_string();
            }
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}
