//! Pure tournament. `route_from_scores` is what `classify` and the recorded
//! tests both run, so a live vector replays without the network.

use std::collections::BTreeMap;

use super::state::Features;
use super::{Decision, DeclineReason, HoldReason, Need, QueryFamily, Route, RouteFlags, Scores};

/// S1. At or above this, hold. On the line-state tune split, ordinary
/// questions peaked at 0.536 and the injection row was 0.669, so the design
/// 0.35 cut held the ordinary mass. 0.60 sits in the gap.
pub const UNSAFE_HOLD: f64 = 0.60;
/// S1 unsure band starts here. Docs only, chain tools off. It sits above the
/// ordinary mass (0.536) and below the hold, so a chain question is not
/// forced onto docs.
pub const UNSAFE_UNSURE: f64 = 0.55;
/// S2 laya pass.
pub const ABOUT_PASS: f64 = 0.70;
/// S2 laya decline. On the tune split, Terp questions sat at or above 0.42
/// and the off-topic japanese row sat at 0.37. Scores in between go to kev.
pub const ABOUT_DECLINE: f64 = 0.40;
/// S2b kev pass.
pub const ABOUT_KEV_PASS: f64 = 0.60;
/// Contrast noul. At or above this, another named chain vetoes a weak Terp score.
pub const OTHER_CHAIN_DECLINE: f64 = 0.60;
/// Laya `other_chain` outside this band is used as-is. Inside it, kev breaks the tie.
pub const OTHER_UNSURE_LO: f64 = 0.30;
pub const OTHER_UNSURE_HI: f64 = 0.70;
/// Financial-advice noul on the legacy walk. At or above this, hold.
/// How-to questions stay below it. The staged walk uses `ADVICE_STAGE`.
pub const ADVICE_HOLD: f64 = 0.60;
/// Staged safety cuts. A lexicon hold still runs first and can only hold.
/// These start above the tune-set safe mass. Lower one only when the tune
/// gap shows every safe row under the cut.
pub const SECRET_HOLD: f64 = 0.90;
/// Above the tune safe max (0.54) and the injection-class max (0.58).
/// The packed noul does not separate, so this cut does not fire. The lexicon holds
/// the injection phrases. The noul is not in the timed pack.
pub const INJECTION_HOLD: f64 = 0.99;
pub const PHISH_HOLD: f64 = 0.72;
pub const IMPERSONATION_HOLD: f64 = 0.85;
pub const HARASS_HOLD: f64 = 0.70;
/// Staged advice cut. Tune advice-class min 0.533, safe max 0.465.
pub const ADVICE_STAGE: f64 = 0.50;
/// A topic noun plus a score at or above this stays on Terp.
pub const ABOUT_STAGE_MIN: f64 = 0.40;
/// No Terp name and no topic noun. The tune off-topic japanese row sat at 0.669
/// and the lowest unnamed on-topic row sat at 0.426 with a topic noun.
pub const ABOUT_STAGE_PASS: f64 = 0.70;
/// Terp named, and no other chain. Below this the name does not keep the question.
/// The tune thiol rows sat at 0.32–0.40. Off-topic rows that name Terp stayed under it.
pub const TERP_NAME_FLOOR: f64 = 0.25;
/// "Needs a human?" Tune human scores start at 0.42 except two at 0.35 and 0.37.
/// Docs never reached 0.39. Live reached 0.41, so 0.40 stole two chain questions.
pub const HUMAN_MIN: f64 = 0.42;
/// docs-vs-live. Live wins only when it clears this and beats docs.
/// 0.55 raised tune live hits from 40 to 46 and did not steal a docs row.
pub const LIVE_MIN: f64 = 0.55;
/// docs-vs-live. Docs wins at or above this. Below both cuts, the branch is mixed.
pub const DOCS_MIN: f64 = 0.80;
/// A docs pick under this, when the question asks for a current chain fact, becomes mixed.
pub const DOCS_LIFT_MAX: f64 = 0.90;
/// S3. Choice confidence at or above this takes the branch. Otherwise mixed.
pub const NEEDS_CONF: f64 = 0.50;
/// S3 bias. A docs answer below this, when the question asks for a current
/// chain fact, becomes mixed. A confident docs answer is left alone.
pub const NEEDS_BIAS_CONF: f64 = 0.65;
/// S4. At or above this, one family. Below, the top two.
pub const FAMILY_CONF: f64 = 0.60;
/// S5. Raw top probability below this also searches the second corpus.
pub const CORPUS_TOP_MIN: f64 = 0.60;
/// Call B version nouls. Both map to `node_versions`.
pub const VERSION_NOUL_MIN: f64 = 0.50;
/// When the tune split shows laya `needs` under 80% at `NEEDS_CONF`, set this
/// so stage 3 is taken from kev instead.
pub const S3_ON_KEV: bool = false;

const NEEDS_OPTIONS: usize = 4;
const FAMILY_OPTIONS: usize = 7;

const S4_NOUNS: &[&str] = &[
    "proposal",
    "proposals",
    "validator",
    "validators",
    "jailed",
    "jail",
    "commission",
    "delegation",
    "delegations",
    "staking",
    "unbonding",
    "ibc",
    "channel",
    "channels",
    "version",
    "upgrade",
    "supply",
    "inflation",
    "balance",
    "denom",
    "contract",
    "wasm",
    "cosmwasm",
    "quorum",
    "tally",
    "mint",
    "slashing",
];

/// Every score the two calls can return. Missing calls stay empty.
/// `route_from_scores` ignores a kev score the production path would not ask for.
#[derive(Debug, Clone, Default)]
pub(super) struct LiveScores {
    pub laya_unsafe: Option<f64>,
    pub laya_advice: Option<f64>,
    pub laya_about: Option<f64>,
    pub laya_other: Option<f64>,
    pub laya_needs: BTreeMap<String, f64>,
    pub laya_corpus: BTreeMap<String, f64>,
    /// Staged safety nouls. Present only on the kev staged call.
    pub secret: Option<f64>,
    pub injection: Option<f64>,
    pub phish: Option<f64>,
    pub impersonation: Option<f64>,
    pub harassment: Option<f64>,
    pub needs_human: Option<f64>,
    pub docs_vs_live: BTreeMap<String, f64>,
    pub kev_about: Option<f64>,
    pub kev_other: Option<f64>,
    pub kev_about_error: bool,
    pub kev_needs: BTreeMap<String, f64>,
    pub family: BTreeMap<String, f64>,
    pub corpus_b: BTreeMap<String, f64>,
    pub ibc_version: Option<f64>,
    pub software_version: Option<f64>,
    pub call_b_error: bool,
}

enum FamilySource {
    /// Stage 4 has not been called. Chain routes set `wants_call_b`.
    Skip,
    Failed,
    Ready,
}

fn s1_blocks(unsafe_p: Option<f64>) -> bool {
    match finite(unsafe_p) {
        None => true,
        Some(p) if p >= UNSAFE_HOLD => true,
        Some(_) => false,
    }
}

fn uses_stages(scores: &LiveScores) -> bool {
    scores.phish.is_some() || scores.needs_human.is_some() || !scores.docs_vs_live.is_empty()
}

pub(super) fn route_from_scores(question: &str, features: &Features, scores: &LiveScores) -> Route {
    if uses_stages(scores) {
        return finish_family(question, features, scores, walk_staged);
    }
    finish_family(question, features, scores, walk)
}

/// Preview used by `classify` to decide whether to spend the family call.
pub(super) fn preview(question: &str, features: &Features, scores: &LiveScores) -> Route {
    if uses_stages(scores) {
        walk_staged(question, features, scores, FamilySource::Skip)
    } else {
        walk(question, features, scores, FamilySource::Skip)
    }
}

fn finish_family(
    question: &str,
    features: &Features,
    scores: &LiveScores,
    step: fn(&str, &Features, &LiveScores, FamilySource) -> Route,
) -> Route {
    let first = step(question, features, scores, FamilySource::Skip);
    if !first.flags.wants_call_b {
        return first;
    }
    let source = if scores.call_b_error || scores.family.is_empty() {
        FamilySource::Failed
    } else {
        FamilySource::Ready
    };
    step(question, features, scores, source)
}

fn walk(question: &str, features: &Features, scores: &LiveScores, source: FamilySource) -> Route {
    let unsafe_p = finite(scores.laya_unsafe);
    let about = finite(scores.laya_about);
    let in_middle = matches!(about, Some(p) if (ABOUT_DECLINE..ABOUT_PASS).contains(&p));
    let used_s2b = !s1_blocks(unsafe_p) && in_middle;
    let base = FlagsBase { used_s2b };

    let Some(unsafe_p) = unsafe_p else {
        return hold(HoldReason::S1Missing, scores, base);
    };
    if unsafe_p >= UNSAFE_HOLD {
        return hold(HoldReason::Unsafe, scores, FlagsBase { used_s2b: false });
    }
    if finite(scores.laya_advice).is_some_and(|p| p >= ADVICE_HOLD) {
        return hold(HoldReason::Advice, scores, FlagsBase { used_s2b: false });
    }
    let docs_only = unsafe_p >= UNSAFE_UNSURE;

    let Some(about) = about else {
        return hold(HoldReason::S2Missing, scores, base);
    };
    let lower = question.to_ascii_lowercase();
    let terp_named = names_terp_subject(&lower);
    let other_named = names_other_chain(&lower);
    // Another chain, and the question does not name Terp, is off-topic even
    // when kev's about score is high.
    if other_named && !terp_named {
        return decline(DeclineReason::OffTopic, scores, base);
    }
    // Terp named and no other chain, or a Terp IBC counterparty. Kev's about
    // score is soft on both. A laya score under ABOUT_DECLINE still declines:
    // naming Terp is not itself an answer.
    let keep = (terp_named && !other_named) || terp_counterparty(question);
    if about < ABOUT_DECLINE {
        return decline(DeclineReason::OffTopic, scores, base);
    }
    if about < ABOUT_PASS {
        if scores.kev_about_error || finite(scores.kev_about).is_none() {
            // A failed tie-break is not evidence the question is off-topic.
            if !keep {
                return hold(HoldReason::S2bFailed, scores, base);
            }
        } else if finite(scores.kev_about).unwrap() < ABOUT_KEV_PASS && !keep {
            return decline(DeclineReason::OffTopic, scores, base);
        }
    }
    let accepted_about = if in_middle {
        finite(scores.kev_about).unwrap_or(about)
    } else {
        about
    };
    if !keep && contrast_veto(about, accepted_about, resolved_other(scores)) {
        return decline(DeclineReason::OffTopic, scores, base);
    }

    let needs = if S3_ON_KEV {
        &scores.kev_needs
    } else {
        &scores.laya_needs
    };
    let (mut need, needs_conf) = pick_need(question, features, needs);
    if docs_only {
        need = Need::Docs;
    }

    let mut flags = RouteFlags {
        docs_only_no_tools: docs_only,
        top2: false,
        chain_degraded: false,
        used_s2b,
        used_call_b: false,
        wants_call_b: false,
    };
    let mut families = Vec::new();
    let mut family_conf = None;

    if docs_only || matches!(need, Need::Docs | Need::Human) {
        // no call B
    } else {
        match source {
            FamilySource::Skip => {
                flags.wants_call_b = true;
            }
            FamilySource::Failed => {
                flags.wants_call_b = true;
                flags.used_call_b = true;
                flags.chain_degraded = true;
                need = Need::Mixed;
                families = QueryFamily::ALL.to_vec();
            }
            FamilySource::Ready => {
                flags.wants_call_b = true;
                flags.used_call_b = true;
                let (picked, top2, conf) = pick_families(
                    &scores.family,
                    question,
                    features,
                    finite(scores.ibc_version),
                    finite(scores.software_version),
                );
                flags.top2 = top2;
                families = picked;
                family_conf = conf;
                if scores.family.is_empty() {
                    flags.chain_degraded = true;
                    flags.top2 = false;
                    need = Need::Mixed;
                    families = QueryFamily::ALL.to_vec();
                }
            }
        }
    }

    let corpora = if matches!(need, Need::Human) {
        Vec::new()
    } else if matches!(need, Need::Chain | Need::Mixed) && !scores.corpus_b.is_empty() {
        rank_corpora(&scores.corpus_b)
    } else {
        rank_corpora(&scores.laya_corpus)
    };

    finish(
        Route {
            decision: Decision::Answer,
            need,
            families,
            corpora,
            flags,
            scores: scores_of(scores, needs_conf, family_conf),
            why: String::new(),
        },
        about_kev_used(scores, used_s2b),
    )
}

/// One kev call already returned safety, topic, and the two needs stages.
/// Family is a second call on the same prefix, and only for chain or mixed.
fn walk_staged(
    question: &str,
    features: &Features,
    scores: &LiveScores,
    source: FamilySource,
) -> Route {
    let base = FlagsBase { used_s2b: false };
    if let Some(reason) = staged_hold(scores) {
        return hold(reason, scores, base);
    }
    let Some(about) = finite(scores.laya_about) else {
        return hold(HoldReason::S2Missing, scores, base);
    };
    let lower = question.to_ascii_lowercase();
    let terp_named = names_terp_subject(&lower);
    let other_named = names_other_chain(&lower);
    if other_named && !terp_named {
        return decline(DeclineReason::OffTopic, scores, base);
    }
    // A Terp IBC counterparty stays on topic even when the score is soft.
    // A named Terp subject stays on topic down to TERP_NAME_FLOOR.
    // Anything else needs a topic noun at ABOUT_STAGE_MIN, or a high score.
    let on_topic = terp_counterparty(question)
        || (terp_named && !other_named && about >= TERP_NAME_FLOOR)
        || about >= ABOUT_STAGE_PASS
        || (about >= ABOUT_STAGE_MIN && has_topic_noun(question));
    if !on_topic {
        return decline(DeclineReason::OffTopic, scores, base);
    }

    let (mut need, needs_conf) = pick_need_staged(question, features, scores);
    let mut flags = RouteFlags {
        docs_only_no_tools: false,
        top2: false,
        chain_degraded: false,
        used_s2b: false,
        used_call_b: false,
        wants_call_b: false,
    };
    let mut families = Vec::new();
    let mut family_conf = None;
    if matches!(need, Need::Docs | Need::Human) {
        // Guides and team questions do not read the chain.
    } else {
        match source {
            FamilySource::Skip => {
                flags.wants_call_b = true;
            }
            FamilySource::Failed => {
                flags.wants_call_b = true;
                flags.used_call_b = true;
                flags.chain_degraded = true;
                need = Need::Mixed;
                families = QueryFamily::ALL.to_vec();
            }
            FamilySource::Ready => {
                flags.wants_call_b = true;
                flags.used_call_b = true;
                let (picked, top2, conf) =
                    pick_families(&scores.family, question, features, None, None);
                flags.top2 = top2;
                families = picked;
                family_conf = conf;
                if scores.family.is_empty() {
                    flags.chain_degraded = true;
                    flags.top2 = false;
                    need = Need::Mixed;
                    families = QueryFamily::ALL.to_vec();
                }
            }
        }
    }
    let corpora = if matches!(need, Need::Human) {
        Vec::new()
    } else {
        rank_corpora(&BTreeMap::new())
    };
    finish(
        Route {
            decision: Decision::Answer,
            need,
            families,
            corpora,
            flags,
            scores: scores_of(scores, needs_conf, family_conf),
            why: String::new(),
        },
        None,
    )
}

fn staged_hold(scores: &LiveScores) -> Option<HoldReason> {
    let checks = [
        (scores.secret, SECRET_HOLD, HoldReason::Secret),
        (scores.phish, PHISH_HOLD, HoldReason::Phish),
        (scores.injection, INJECTION_HOLD, HoldReason::Injection),
        (
            scores.impersonation,
            IMPERSONATION_HOLD,
            HoldReason::Impersonation,
        ),
        (scores.harassment, HARASS_HOLD, HoldReason::Harassment),
        (scores.laya_advice, ADVICE_STAGE, HoldReason::Advice),
    ];
    for (score, cut, reason) in checks {
        if finite(score).is_some_and(|p| p >= cut) {
            return Some(reason);
        }
    }
    None
}

fn pick_need_staged(
    question: &str,
    features: &Features,
    scores: &LiveScores,
) -> (Need, Option<f64>) {
    if finite(scores.needs_human).is_some_and(|p| p >= HUMAN_MIN) {
        return (Need::Human, finite(scores.needs_human));
    }
    let docs_p = scores
        .docs_vs_live
        .get("docs")
        .copied()
        .filter(|p| p.is_finite())
        .unwrap_or(0.0);
    let live_p = scores
        .docs_vs_live
        .get("live_chain")
        .copied()
        .filter(|p| p.is_finite())
        .unwrap_or(0.0);
    let mut need = if live_p >= LIVE_MIN && live_p >= docs_p {
        Need::Chain
    } else if docs_p >= DOCS_MIN {
        Need::Docs
    } else {
        Need::Mixed
    };
    if need == Need::Docs && (features.proposal_id.is_some() || features.has_address) {
        need = Need::Mixed;
    }
    if need == Need::Docs
        && features.asks_current
        && has_word(question, S4_NOUNS)
        && docs_p < DOCS_LIFT_MAX
    {
        need = Need::Mixed;
    }
    let conf = match need {
        Need::Chain => Some(live_p),
        Need::Docs => Some(docs_p),
        Need::Human => finite(scores.needs_human),
        Need::Mixed => Some(docs_p.max(live_p)),
    };
    (need, conf)
}

struct FlagsBase {
    used_s2b: bool,
}

fn hold(reason: HoldReason, scores: &LiveScores, base: FlagsBase) -> Route {
    finish(
        Route {
            decision: Decision::Hold { reason },
            need: Need::Docs,
            families: Vec::new(),
            corpora: Vec::new(),
            flags: empty_flags(base.used_s2b),
            scores: scores_of(scores, None, None),
            why: String::new(),
        },
        finite(scores.kev_about).filter(|_| base.used_s2b),
    )
}

fn decline(reason: DeclineReason, scores: &LiveScores, base: FlagsBase) -> Route {
    finish(
        Route {
            decision: Decision::Decline { reason },
            need: Need::Docs,
            families: Vec::new(),
            corpora: Vec::new(),
            flags: empty_flags(base.used_s2b),
            scores: scores_of(scores, None, None),
            why: String::new(),
        },
        finite(scores.kev_about).filter(|_| base.used_s2b),
    )
}

fn empty_flags(used_s2b: bool) -> RouteFlags {
    RouteFlags {
        docs_only_no_tools: false,
        top2: false,
        chain_degraded: false,
        used_s2b,
        used_call_b: false,
        wants_call_b: false,
    }
}

fn about_kev_used(scores: &LiveScores, used_s2b: bool) -> Option<f64> {
    finite(scores.kev_about).filter(|_| used_s2b)
}

/// Kev's `other_chain` separates Terp from another chain. Laya's score sits
/// high on both, so it is used only when kev did not answer.
fn resolved_other(scores: &LiveScores) -> Option<f64> {
    if let Some(kev) = finite(scores.kev_other) {
        return Some(kev);
    }
    let laya = finite(scores.laya_other)?;
    if !(OTHER_UNSURE_LO..OTHER_UNSURE_HI).contains(&laya) {
        Some(laya)
    } else {
        None
    }
}

fn has_token(lower: &str, token: &str) -> bool {
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word == token)
}

const OTHER_CHAIN_NAMES: &[&str] = &[
    "cosmos hub",
    "osmosis",
    "juno",
    "neutron",
    "stride",
    "akash",
    "ethereum",
    "bitcoin",
    "gaia",
    "secret network",
    "metamask",
];

fn names_other_chain(lower: &str) -> bool {
    OTHER_CHAIN_NAMES.iter().any(|name| lower.contains(name))
        || lower.contains("the hub")
        || has_token(lower, "atom")
        || has_token(lower, "eth")
}

/// The Terp chain or the THIOL denom. "terpene" and "thiol group" do not match.
fn names_terp_subject(lower: &str) -> bool {
    if has_token(lower, "terp") || lower.contains("morocco") {
        return true;
    }
    has_token(lower, "thiol") && !lower.contains("thiol group")
}

/// A Terp IBC counterparty, or a comparison that asks about Terp.
/// Kev scores these like the other chain. The labeling rule still answers them.
fn terp_counterparty(question: &str) -> bool {
    let lower = question.to_ascii_lowercase();
    if !names_terp_subject(&lower) || !names_other_chain(&lower) {
        return false;
    }
    const REL: &[&str] = &[
        "ibc", "channel", "channels", "transfer", "relayer", "differ", "compare", "versus", "vs",
        "newer",
    ];
    REL.iter().any(|word| has_token(&lower, word))
}

/// Another chain vetoes only when Terp itself was not confidently the subject.
/// A high kev Terp score still wins when it is at least as strong as the veto.
fn contrast_veto(laya_about: f64, accepted_about: f64, other: Option<f64>) -> bool {
    let Some(other) = other else {
        return false;
    };
    if other < OTHER_CHAIN_DECLINE {
        return false;
    }
    if laya_about >= ABOUT_PASS {
        return false;
    }
    if accepted_about >= ABOUT_KEV_PASS && accepted_about >= other {
        return false;
    }
    true
}

fn finish(mut route: Route, kev_about: Option<f64>) -> Route {
    let need = format!("{:?}", route.need).to_lowercase();
    let family = route
        .scores
        .family
        .iter()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(label, _)| label.as_str())
        .unwrap_or("-");
    route.why = format!(
        "unsafe {} advice {} other {} about_terp laya {}→kev {} human {} needs {need}@{} family {family}@{}",
        fmt_p(route.scores.unsafe_p),
        fmt_p(route.scores.advice),
        fmt_p(route.scores.other_chain),
        fmt_p(route.scores.about_terp),
        fmt_p(kev_about),
        fmt_p(route.scores.needs_human),
        fmt_p(route.scores.needs_conf),
        fmt_p(route.scores.family_conf),
    );
    route
}

fn fmt_p(p: Option<f64>) -> String {
    match p {
        Some(p) => format!("{p:.2}"),
        None => "-".to_string(),
    }
}

fn scores_of(scores: &LiveScores, needs_conf: Option<f64>, family_conf: Option<f64>) -> Scores {
    let needs = if !scores.docs_vs_live.is_empty() {
        scores.docs_vs_live.clone()
    } else if S3_ON_KEV && !scores.kev_needs.is_empty() {
        scores.kev_needs.clone()
    } else {
        scores.laya_needs.clone()
    };
    Scores {
        unsafe_p: finite(scores.laya_unsafe),
        about_terp: finite(scores.laya_about),
        about_terp_kev: finite(scores.kev_about),
        needs_conf,
        family_conf,
        advice: finite(scores.laya_advice),
        other_chain: resolved_other(scores),
        needs_human: finite(scores.needs_human),
        needs,
        family: scores.family.clone(),
        corpus: if !scores.corpus_b.is_empty() {
            scores.corpus_b.clone()
        } else {
            scores.laya_corpus.clone()
        },
    }
}

fn pick_need(
    question: &str,
    features: &Features,
    needs: &BTreeMap<String, f64>,
) -> (Need, Option<f64>) {
    let Some((label, _top, conf)) = top_conf(needs, NEEDS_OPTIONS) else {
        return (Need::Mixed, None);
    };
    let mut need = match label.as_str() {
        "docs" => Need::Docs,
        "live_chain" => Need::Chain,
        "human" => Need::Human,
        // `mixed` and any unknown label both take the safe branch.
        _ => Need::Mixed,
    };
    if conf < NEEDS_CONF
        || (need == Need::Docs
            && features.asks_current
            && has_word(question, S4_NOUNS)
            && conf < NEEDS_BIAS_CONF)
    {
        need = Need::Mixed;
    }
    if need == Need::Docs && (features.proposal_id.is_some() || features.has_address) {
        need = Need::Mixed;
    }
    (need, Some(conf))
}

fn pick_families(
    probs: &BTreeMap<String, f64>,
    question: &str,
    features: &Features,
    ibc_version: Option<f64>,
    software_version: Option<f64>,
) -> (Vec<QueryFamily>, bool, Option<f64>) {
    let ranked = rank_desc(probs);
    if ranked.is_empty() {
        return (QueryFamily::ALL.to_vec(), false, None);
    }
    let conf = choice_confidence(ranked[0].1, FAMILY_OPTIONS);
    let top2 = conf < FAMILY_CONF;
    let n = if top2 { 2 } else { 1 };
    let mut families = Vec::new();
    for (label, _) in ranked.into_iter().take(n) {
        for family in tools_for(&label, question, features) {
            push_unique(&mut families, family);
        }
    }
    if ibc_version.is_some_and(|p| p >= VERSION_NOUL_MIN)
        || software_version.is_some_and(|p| p >= VERSION_NOUL_MIN)
    {
        push_unique(&mut families, QueryFamily::NodeVersion);
    }
    (families, top2, Some(conf))
}

fn tools_for(label: &str, question: &str, features: &Features) -> Vec<QueryFamily> {
    let mut out = Vec::new();
    match label {
        "gov" => {
            if features.proposal_id.is_some() {
                push_unique(&mut out, QueryFamily::GovProposal);
                push_unique(&mut out, QueryFamily::GovTally);
            } else if has_word(question, &["quorum", "deposit", "tallying"]) {
                push_unique(&mut out, QueryFamily::GovParams);
            } else if has_word(question, &["voting"]) && features.asks_current {
                push_unique(&mut out, QueryFamily::GovActive);
            } else {
                push_unique(&mut out, QueryFamily::GovLatest);
            }
        }
        "staking" => {
            if features.has_valoper || has_word(question, &["jailed", "jail"]) {
                push_unique(&mut out, QueryFamily::Validator);
            } else if has_word(question, &["unbonding", "commission", "params", "param"]) {
                push_unique(&mut out, QueryFamily::StakingParams);
            } else if has_word(question, &["slashing", "slash", "misses", "miss"]) {
                push_unique(&mut out, QueryFamily::SlashingParams);
            } else if has_word(question, &["pool", "bonded"]) {
                push_unique(&mut out, QueryFamily::StakingPool);
            } else {
                push_unique(&mut out, QueryFamily::Validators);
            }
        }
        "ibc" => {
            if has_word(question, &["client", "clients"]) {
                push_unique(&mut out, QueryFamily::IbcClients);
            }
            if has_word(question, &["connection", "connections"]) {
                push_unique(&mut out, QueryFamily::IbcConnections);
            }
            if out.is_empty() || has_word(question, &["channel", "channels", "connected", "chains"])
            {
                push_unique(&mut out, QueryFamily::IbcChannels);
            }
            if has_word(question, &["version"]) || question.to_ascii_lowercase().contains("ibc-go")
            {
                push_unique(&mut out, QueryFamily::NodeVersion);
            }
        }
        "upgrade_version" => {
            if has_word(question, &["upgrade", "upcoming", "plan"]) {
                push_unique(&mut out, QueryFamily::UpgradeCurrent);
            }
            if has_word(question, &["applied"]) {
                push_unique(&mut out, QueryFamily::UpgradeApplied);
            }
            if out.is_empty()
                || has_word(
                    question,
                    &["version", "sdk", "wasmd", "ibc", "software", "terpd"],
                )
            {
                push_unique(&mut out, QueryFamily::NodeVersion);
            }
        }
        "bank_mint" => {
            if features.has_address && !features.has_valoper {
                push_unique(&mut out, QueryFamily::Balance);
            } else if has_word(question, &["inflation", "mint"]) {
                push_unique(&mut out, QueryFamily::Mint);
            } else if has_word(question, &["gas"]) {
                push_unique(&mut out, QueryFamily::Mint);
                push_unique(&mut out, QueryFamily::Supply);
            } else {
                push_unique(&mut out, QueryFamily::Supply);
            }
        }
        "contract" => push_unique(&mut out, QueryFamily::Contract),
        _ => {}
    }
    out
}

pub(super) fn choice_confidence(top: f64, n: usize) -> f64 {
    if n <= 1 {
        return 1.0;
    }
    let n = n as f64;
    let base = 1.0 / n;
    let denom = 1.0 - base;
    if denom <= f64::EPSILON {
        return 0.0;
    }
    ((top - base) / denom).clamp(0.0, 1.0)
}

fn top_conf(probs: &BTreeMap<String, f64>, n_options: usize) -> Option<(String, f64, f64)> {
    let ranked = rank_desc(probs);
    let (label, top) = ranked.into_iter().next()?;
    let n = n_options.max(1);
    Some((label, top, choice_confidence(top, n)))
}

fn rank_desc(probs: &BTreeMap<String, f64>) -> Vec<(String, f64)> {
    let mut items: Vec<(String, f64)> = probs
        .iter()
        .filter(|(_, p)| p.is_finite())
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    items.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    items
}

fn rank_corpora(probs: &BTreeMap<String, f64>) -> Vec<String> {
    let ranked = rank_desc(probs);
    if ranked.is_empty() {
        return vec!["terp-docs".into(), "terp-core".into(), "terp".into()];
    }
    if ranked[0].1 < CORPUS_TOP_MIN {
        ranked.into_iter().take(2).map(|(k, _)| k).collect()
    } else {
        vec![ranked.into_iter().next().unwrap().0]
    }
}

fn push_unique(out: &mut Vec<QueryFamily>, family: QueryFamily) {
    if !out.contains(&family) {
        out.push(family);
    }
}

/// Chain words that are not the Terp name. Kept in step with `TOPIC_NOUNS` in
/// `scripts/gate_eval.py`. `S4_NOUNS` is included via `has_topic_noun`.
const TOPIC_NOUNS: &[&str] = &[
    "headstash",
    "thiol",
    "uthiol",
    "dao",
    "wallet",
    "node",
    "stake",
    "staking",
    "faucet",
    "speakeasy",
    "morocco",
    "keplr",
    "gas",
    "token",
    "airdrop",
    "commission",
    "slash",
    "jail",
    "delegate",
    "mnemonic",
    "seed",
];

fn has_topic_noun(question: &str) -> bool {
    has_word(question, S4_NOUNS) || has_word(question, TOPIC_NOUNS)
}

fn has_word(question: &str, words: &[&str]) -> bool {
    question
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .any(|word| words.contains(&word))
}

fn finite(p: Option<f64>) -> Option<f64> {
    p.filter(|n| n.is_finite())
}

#[cfg(test)]
mod tests {
    use super::super::state::extract_features;
    use super::*;

    fn map(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    fn scores_for(unsafe_p: f64, about: f64, needs: &[(&str, f64)]) -> LiveScores {
        LiveScores {
            laya_unsafe: Some(unsafe_p),
            laya_about: Some(about),
            laya_needs: map(needs),
            ..LiveScores::default()
        }
    }

    fn decide_q(question: &str, scores: LiveScores) -> Route {
        route_from_scores(question, &extract_features(question), &scores)
    }

    #[test]
    fn confidence_matches_the_formula() {
        let conf = choice_confidence(0.625, 4);
        assert!((conf - 0.5).abs() < 1e-9);
        assert!(choice_confidence(0.70, 4) > 0.5);
    }

    #[test]
    fn ibc_version_is_chain_and_reads_node_info() {
        let question = "what version of ibc is terp network on";
        let mut scores = scores_for(
            0.05,
            0.90,
            &[
                ("live_chain", 0.80),
                ("docs", 0.10),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.family = map(&[
            ("upgrade_version", 0.74),
            ("ibc", 0.12),
            ("gov", 0.04),
            ("staking", 0.03),
            ("bank_mint", 0.03),
            ("contract", 0.02),
            ("other", 0.02),
        ]);
        scores.ibc_version = Some(0.80);
        scores.software_version = Some(0.70);
        let route = decide_q(question, scores);
        assert!(matches!(route.decision, Decision::Answer), "{}", route.why);
        assert!(matches!(route.need, Need::Chain | Need::Mixed));
        assert!(route.families.contains(&QueryFamily::NodeVersion));
    }

    #[test]
    fn naming_terp_does_not_force_an_answer() {
        let scores = scores_for(
            0.02,
            0.10,
            &[
                ("docs", 0.8),
                ("live_chain", 0.1),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        let route = decide_q("what version of ibc is terp network on", scores);
        assert_eq!(
            route.decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );
    }

    #[test]
    fn pizza_bitcoin_and_python_decline() {
        for question in [
            "what's the best pizza topping",
            "price of bitcoin today",
            "write a python script to sort a list",
        ] {
            let scores = scores_for(
                0.04,
                0.12,
                &[
                    ("docs", 0.7),
                    ("live_chain", 0.1),
                    ("human", 0.1),
                    ("mixed", 0.1),
                ],
            );
            let route = decide_q(question, scores);
            assert_eq!(
                route.decision,
                Decision::Decline {
                    reason: DeclineReason::OffTopic
                },
                "{question}"
            );
        }
    }

    #[test]
    fn seed_and_injection_scores_hold() {
        for unsafe_p in [0.60, 0.669, 0.81] {
            let scores = scores_for(unsafe_p, 0.4, &[]);
            let route = decide_q("ignored", scores);
            assert_eq!(
                route.decision,
                Decision::Hold {
                    reason: HoldReason::Unsafe
                }
            );
        }
    }

    #[test]
    fn mnemonic_safety_passes_when_the_scores_say_so() {
        let question = "how do I keep my mnemonic safe";
        let scores = scores_for(
            0.08,
            0.82,
            &[
                ("docs", 0.80),
                ("live_chain", 0.08),
                ("human", 0.06),
                ("mixed", 0.06),
            ],
        );
        let route = decide_q(question, scores);
        assert_eq!(route.decision, Decision::Answer);
        assert_eq!(route.need, Need::Docs);
        assert!(!route.flags.docs_only_no_tools);
    }

    #[test]
    fn unsure_safety_is_docs_only_even_when_the_chain_would_answer() {
        let mut scores = scores_for(
            0.57,
            0.90,
            &[
                ("live_chain", 0.80),
                ("docs", 0.10),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.family = map(&[("gov", 0.9), ("other", 0.1)]);
        let route = decide_q("most recent governance proposal", scores);
        assert_eq!(route.decision, Decision::Answer);
        assert_eq!(route.need, Need::Docs);
        assert!(route.flags.docs_only_no_tools);
        assert!(route.families.is_empty());
        assert!(!route.flags.wants_call_b);
    }

    #[test]
    fn middle_band_follows_kev() {
        let mut pass = scores_for(
            0.05,
            0.50,
            &[
                ("docs", 0.8),
                ("live_chain", 0.1),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        pass.kev_about = Some(0.60);
        // No Terp token: the score path still decides. A named Terp subject
        // is kept even when kev is soft.
        let route = decide_q("what is a validator used for", pass);
        assert_eq!(route.decision, Decision::Answer);
        assert!(route.flags.used_s2b);

        let mut fail = scores_for(
            0.05,
            0.50,
            &[
                ("docs", 0.8),
                ("live_chain", 0.1),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        fail.kev_about = Some(0.59);
        assert_eq!(
            decide_q("what is a validator used for", fail).decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );

        let mut down = scores_for(
            0.05,
            0.50,
            &[
                ("docs", 0.8),
                ("live_chain", 0.1),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        down.kev_about_error = true;
        assert_eq!(
            decide_q("what is a validator used for", down).decision,
            Decision::Hold {
                reason: HoldReason::S2bFailed
            }
        );
    }

    #[test]
    fn low_needs_confidence_and_call_b_failure_degrade_to_mixed() {
        let weak = scores_for(
            0.05,
            0.9,
            &[
                ("docs", 0.40),
                ("live_chain", 0.30),
                ("human", 0.15),
                ("mixed", 0.15),
            ],
        );
        let route = decide_q("what is THIOL used for", weak);
        assert_eq!(route.need, Need::Mixed);
        assert!(route.flags.chain_degraded);
        assert_eq!(route.families.len(), QueryFamily::ALL.len());

        let mut failed = scores_for(
            0.05,
            0.9,
            &[
                ("live_chain", 0.80),
                ("docs", 0.10),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        failed.call_b_error = true;
        let route = decide_q("most recent governance proposal", failed);
        assert_eq!(route.need, Need::Mixed);
        assert!(route.flags.chain_degraded);
    }

    #[test]
    fn proposal_id_lifts_docs_to_mixed_and_current_only_biases() {
        let docs = [
            ("docs", 0.90),
            ("live_chain", 0.04),
            ("human", 0.03),
            ("mixed", 0.03),
        ];
        let mut with_family = scores_for(0.05, 0.9, &docs);
        with_family.family = map(&[("gov", 0.8), ("other", 0.2)]);
        let lifted = decide_q("what is proposal 62 about and did it pass", with_family);
        assert_eq!(lifted.need, Need::Mixed);
        assert!(lifted.families.contains(&QueryFamily::GovProposal));

        let mut soft = scores_for(
            0.05,
            0.9,
            &[
                ("docs", 0.70),
                ("live_chain", 0.20),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        soft.family = map(&[("upgrade_version", 0.8), ("other", 0.2)]);
        let biased = decide_q("what version of terpd should I run", soft);
        assert_eq!(biased.need, Need::Mixed);

        let mut firm = scores_for(0.05, 0.9, &docs);
        firm.family = map(&[("upgrade_version", 0.8), ("other", 0.2)]);
        let kept = decide_q("what version of terpd should I run", firm);
        assert_eq!(
            kept.need,
            Need::Docs,
            "a confident docs answer is not forced"
        );
    }

    #[test]
    fn low_family_confidence_keeps_the_top_two() {
        let mut scores = scores_for(
            0.05,
            0.9,
            &[
                ("live_chain", 0.80),
                ("docs", 0.10),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.family = map(&[
            ("gov", 0.30),
            ("staking", 0.25),
            ("ibc", 0.15),
            ("upgrade_version", 0.10),
            ("bank_mint", 0.08),
            ("contract", 0.07),
            ("other", 0.05),
        ]);
        let route = decide_q("most recent governance proposal", scores);
        assert!(route.flags.top2);
        assert!(route.families.contains(&QueryFamily::GovLatest));
        assert!(route.families.contains(&QueryFamily::Validators));
    }

    #[test]
    fn contrast_veto_declines_when_kev_calls_the_hub_terp() {
        let mut scores = scores_for(
            0.05,
            0.52,
            &[
                ("docs", 0.70),
                ("live_chain", 0.20),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.kev_about = Some(0.74);
        scores.laya_other = Some(0.40);
        scores.kev_other = Some(0.82);
        let route = decide_q("how do validators work on the cosmos hub", scores);
        assert_eq!(
            route.decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );
    }

    #[test]
    fn contrast_does_not_veto_a_confident_terp_question() {
        let mut scores = scores_for(
            0.05,
            0.88,
            &[
                ("live_chain", 0.80),
                ("docs", 0.10),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.laya_other = Some(0.80);
        scores.family = map(&[("ibc", 0.80), ("other", 0.20)]);
        let route = decide_q("what's the IBC channel from terp to osmosis", scores);
        assert_eq!(route.decision, Decision::Answer);
        assert!(matches!(route.need, Need::Chain | Need::Mixed));
    }

    #[test]
    fn lone_laya_other_does_not_veto_when_kev_says_terp() {
        let mut scores = scores_for(
            0.26,
            0.58,
            &[
                ("docs", 0.70),
                ("live_chain", 0.20),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.kev_about = Some(0.92);
        scores.laya_other = Some(0.76);
        scores.kev_other = Some(0.08);
        let route = decide_q("what is the purpose of validators on terp network", scores);
        assert_eq!(route.decision, Decision::Answer);
    }

    #[test]
    fn terp_ibc_counterparty_answers_when_kev_about_is_low() {
        let mut scores = scores_for(
            0.26,
            0.56,
            &[
                ("live_chain", 0.80),
                ("docs", 0.10),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.kev_about = Some(0.33);
        scores.laya_other = Some(0.69);
        scores.kev_other = Some(0.91);
        scores.family = map(&[("ibc", 0.80), ("other", 0.20)]);
        let route = decide_q(
            "what is the IBC channel from terp to the cosmos hub",
            scores,
        );
        assert_eq!(route.decision, Decision::Answer);
        assert!(matches!(route.need, Need::Chain | Need::Mixed));
    }

    #[test]
    fn a_named_other_chain_declines_even_when_kev_says_terp() {
        let mut scores = scores_for(
            0.20,
            0.57,
            &[
                ("docs", 0.70),
                ("live_chain", 0.20),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.kev_about = Some(0.80);
        scores.kev_other = Some(0.20);
        let route = decide_q("how do I bridge ETH on metamask", scores);
        assert_eq!(
            route.decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );
    }

    #[test]
    fn a_terp_named_question_survives_a_soft_kev_score() {
        let mut scores = scores_for(
            0.20,
            0.55,
            &[
                ("docs", 0.80),
                ("live_chain", 0.10),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.kev_about = Some(0.46);
        let route = decide_q("para que serve o thiol na terp", scores);
        assert_eq!(route.decision, Decision::Answer);
    }

    #[test]
    fn terpene_and_thiol_group_are_not_terp() {
        let mut soft = scores_for(
            0.20,
            0.55,
            &[
                ("docs", 0.80),
                ("live_chain", 0.10),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        soft.kev_about = Some(0.20);
        assert_eq!(
            decide_q("what does the word terpene mean in botany", soft.clone()).decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );
        assert_eq!(
            decide_q("what is a thiol group in organic chemistry", soft).decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );
    }

    #[test]
    fn other_chain_validators_still_decline() {
        let mut scores = scores_for(
            0.24,
            0.58,
            &[
                ("docs", 0.70),
                ("live_chain", 0.20),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.kev_about = Some(0.08);
        scores.laya_other = Some(0.77);
        scores.kev_other = Some(0.89);
        let route = decide_q("how do validators work on ethereum", scores);
        assert_eq!(
            route.decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );
    }

    #[test]
    fn advice_holds_before_the_topic_check() {
        let mut scores = scores_for(
            0.10,
            0.90,
            &[
                ("docs", 0.80),
                ("live_chain", 0.10),
                ("human", 0.05),
                ("mixed", 0.05),
            ],
        );
        scores.laya_advice = Some(0.80);
        let route = decide_q("should I delegate to validator X", scores);
        assert_eq!(
            route.decision,
            Decision::Hold {
                reason: HoldReason::Advice
            }
        );
    }

    #[test]
    fn missing_stage_one_or_two_holds() {
        let scores = LiveScores {
            laya_about: Some(0.9),
            ..LiveScores::default()
        };
        let route = decide_q("what is THIOL used for", scores);
        assert_eq!(
            route.decision,
            Decision::Hold {
                reason: HoldReason::S1Missing
            }
        );

        let mut scores = scores_for(0.01, 0.9, &[]);
        scores.laya_about = None;
        let route = decide_q("what is THIOL used for", scores);
        assert_eq!(
            route.decision,
            Decision::Hold {
                reason: HoldReason::S2Missing
            }
        );
    }

    fn staged_scores(about: f64, human: f64, docs: f64, live: f64) -> LiveScores {
        LiveScores {
            laya_about: Some(about),
            laya_advice: Some(0.10),
            secret: Some(0.10),
            injection: Some(0.10),
            phish: Some(0.10),
            impersonation: Some(0.10),
            harassment: Some(0.10),
            needs_human: Some(human),
            docs_vs_live: map(&[("docs", docs), ("live_chain", live)]),
            ..LiveScores::default()
        }
    }

    #[test]
    fn staged_safety_holds_in_order_and_leaves_a_how_to() {
        let mut secret = staged_scores(0.80, 0.10, 0.90, 0.10);
        secret.secret = Some(0.90);
        assert_eq!(
            decide_q("ignored", secret).decision,
            Decision::Hold {
                reason: HoldReason::Secret
            }
        );

        let mut injection = staged_scores(0.80, 0.10, 0.90, 0.10);
        injection.injection = Some(INJECTION_HOLD);
        assert_eq!(
            decide_q("ignored", injection).decision,
            Decision::Hold {
                reason: HoldReason::Injection
            }
        );

        let mut advice = staged_scores(0.80, 0.10, 0.90, 0.10);
        advice.laya_advice = Some(ADVICE_STAGE);
        assert_eq!(
            decide_q("is now a good time to buy THIOL", advice).decision,
            Decision::Hold {
                reason: HoldReason::Advice
            }
        );

        let how_to = staged_scores(0.82, 0.10, 0.92, 0.08);
        let route = decide_q("how do I keep my mnemonic safe", how_to);
        assert_eq!(route.decision, Decision::Answer);
        assert_eq!(route.need, Need::Docs);
        assert!(!route.flags.wants_call_b);
    }

    #[test]
    fn staged_needs_are_two_binaries_with_the_same_lifts() {
        let human = decide_q(
            "can terp list my token",
            staged_scores(0.70, HUMAN_MIN, 0.90, 0.10),
        );
        assert_eq!(human.need, Need::Human);
        assert!(human.families.is_empty());
        assert!(!human.flags.wants_call_b);

        let mut chain = staged_scores(0.80, 0.10, 0.20, LIVE_MIN);
        chain.family = map(&[("ibc", 0.80), ("other", 0.20)]);
        let chain = decide_q("what version of ibc is terp network on", chain);
        assert_eq!(chain.need, Need::Chain);
        assert!(chain.families.contains(&QueryFamily::NodeVersion));
        assert!(chain.families.contains(&QueryFamily::IbcChannels));

        let proposal = decide_q(
            "what is proposal 62 about and did it pass",
            staged_scores(0.80, 0.10, 0.95, 0.05),
        );
        assert_eq!(proposal.need, Need::Mixed);

        let upcoming = decide_q(
            "is there an upcoming chain upgrade",
            staged_scores(0.70, 0.10, 0.85, 0.15),
        );
        assert_eq!(upcoming.need, Need::Mixed);

        let guide = decide_q(
            "how do I upgrade my node with cosmovisor",
            staged_scores(0.90, 0.10, 0.96, 0.04),
        );
        assert_eq!(guide.need, Need::Docs);
    }

    #[test]
    fn staged_topic_keeps_a_named_terp_and_a_failed_family_degrades() {
        let named = decide_q(
            "what version of ibc is terp network on",
            staged_scores(0.39, 0.10, 0.90, 0.10),
        );
        assert_eq!(named.decision, Decision::Answer);

        let below_floor = decide_q(
            "what version of ibc is terp network on",
            staged_scores(0.10, 0.10, 0.90, 0.10),
        );
        assert_eq!(
            below_floor.decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );

        let other = decide_q(
            "how do validators work on ethereum",
            staged_scores(0.80, 0.10, 0.90, 0.10),
        );
        assert_eq!(
            other.decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );

        let prose = decide_q(
            "translate this paragraph into japanese",
            staged_scores(0.66, 0.10, 0.90, 0.10),
        );
        assert_eq!(
            prose.decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );

        let noun = decide_q(
            "why is my validator jailed",
            staged_scores(0.50, 0.10, 0.90, 0.10),
        );
        assert_eq!(noun.decision, Decision::Answer);

        let soft_noun = decide_q(
            "why is my validator jailed",
            staged_scores(0.39, 0.10, 0.90, 0.10),
        );
        assert_eq!(
            soft_noun.decision,
            Decision::Decline {
                reason: DeclineReason::OffTopic
            }
        );

        let counterparty = decide_q(
            "what is the IBC channel from terp to osmosis",
            staged_scores(0.29, 0.10, 0.20, 0.80),
        );
        assert_eq!(counterparty.decision, Decision::Answer);
        assert!(matches!(counterparty.need, Need::Chain | Need::Mixed));

        let mut failed = staged_scores(0.80, 0.10, 0.10, 0.90);
        failed.call_b_error = true;
        let route = decide_q("most recent governance proposal", failed);
        assert_eq!(route.need, Need::Mixed);
        assert!(route.flags.chain_degraded);

        let missing = LiveScores {
            phish: Some(0.10),
            needs_human: Some(0.10),
            ..LiveScores::default()
        };
        assert_eq!(
            decide_q("what is a validator", missing).decision,
            Decision::Hold {
                reason: HoldReason::S2Missing
            }
        );
    }
}
