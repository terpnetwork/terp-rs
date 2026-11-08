//! Anti-spam: Rust port of DA0-DA0 `discord-anti-spam-bot/bot.py`, hardened.
//!
//! Two upgrades over the original:
//! - **Confusable normalization**: crypto spam uses Cyrillic/Greek homoglyphs
//!   (`Оsmоsis`, `hаs`, `Аirdrоp`) to dodge keyword matchers. Every message
//!   is folded to Latin lookalikes before matching.
//! - **Airdrop-pattern rules**: gift/party emoji + `airdrop`/`claim` + a
//!   chain or token name (osmosis, ion, astrovault, axvdex, …) is spam even
//!   with no URL. Known scam domains stay keyword-matched.
//!
//! Action is delete + long timeout + notify channel. Env: `SPAM_KEYWORDS`
//! (comma-sep, appended to the built-in scam list), `SPAM_NOTIFY_CHANNEL_ID`,
//! `SPAM_TIMEOUT_DAYS` (default 27, capped at Discord's 28).

/// Cyrillic + Greek lookalikes folded to Latin. Covers the homoglyphs seen
/// in the wild (`Оsmоsis hаs lаunchеd its sеcоnd IОN Аirdrоp!`).
fn fold_confusables(c: char) -> char {
    match c {
        // Cyrillic АВЕКМНОРСТХ / аеорссуху  → Latin
        'А' | 'Α' => 'A',
        'В' => 'B',
        'Е' | 'Ε' => 'E',
        'Ё' => 'E',
        'К' | 'Κ' => 'K',
        'М' | 'Μ' => 'M',
        'Н' | 'Η' => 'H',
        'О' | 'Ο' => 'O',
        'Р' | 'Ρ' => 'P',
        'С' => 'C',
        'Т' | 'Τ' => 'T',
        'Х' | 'Χ' => 'X',
        'а' => 'a',
        'с' | 'ϲ' => 'c',
        'е' | 'ε' => 'e',
        'ё' => 'e',
        'і' | 'ӏ' => 'i',
        'ј' => 'j',
        'о' | 'ο' => 'o',
        'р' | 'ρ' => 'p',
        'ѕ' => 's',
        'х' | 'χ' => 'x',
        'у' | 'γ' => 'y',
        // Misc confusables
        'ɑ' => 'a',
        'ɡ' => 'g',
        '𝐚'..='𝐳' | '𝑎'..='𝑧' | '𝒂'..='𝒛' => {
            char::from_u32('a' as u32 + (c as u32 - first_of(c))).unwrap_or(c)
        }
        '𝐀'..='𝐙' | '𝐴'..='𝑍' | '𝒜'..='𝒵' => {
            char::from_u32('A' as u32 + (c as u32 - first_of(c))).unwrap_or(c)
        }
        _ => c,
    }
}

fn first_of(c: char) -> u32 {
    for base in ['𝐚', '𝑎', '𝒂', '𝐀', '𝐴', '𝒜'] {
        if (c as u32) >= base as u32 && (c as u32) < base as u32 + 26 {
            return base as u32;
        }
    }
    c as u32
}

/// Lowercase + confusable-folded message text for matching.
pub fn normalize(content: &str) -> String {
    content.chars().map(fold_confusables).collect::<String>().to_lowercase()
}

pub fn keywords() -> Vec<String> {
    let mut base = vec![
        "axvdex.org".to_string(),
        "astrovault airdrop".to_string(),
        "ion airdrop".to_string(),
        "osmosis airdrop".to_string(),
    ];
    if let Ok(v) = std::env::var("SPAM_KEYWORDS") {
        for k in v.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
            if !base.contains(&k) {
                base.push(k);
            }
        }
    }
    base
}

fn has_gift_emoji(c: &str) -> bool {
    c.chars().any(|ch| matches!(ch,
        '🎁' | '🎉' | '🎊' | '💰' | '💸' | '🪂' | '🚀' | '✅' | '‼' | '❗' | '⬇'
    ))
}

fn has_chain_or_token(c: &str) -> bool {
    ["osmosis", "ion", "astrovault", "axvdex", "cosmos", "terra", "injective", "celestia", "dydx", "stargaze"]
        .iter()
        .any(|t| c.contains(t))
}

pub fn is_spam(content: &str) -> bool {
    let c = normalize(content);
    if keywords().iter().any(|k| c.contains(k)) {
        return true;
    }
    // Gift-wrapped airdrop announcement: emoji + airdrop/claim + known
    // chain/token name. This is the exact shape of the ION wave.
    if (c.contains("airdrop") || c.contains("air drop"))
        && (c.contains("claim") || c.contains("launch") || has_gift_emoji(content))
        && has_chain_or_token(&c)
    {
        return true;
    }
    // Bare "claim your X" with URL-ish bait and urgency words.
    if c.contains("claim") && (c.contains("http") || c.contains(".org") || c.contains(".io") || c.contains(".xyz"))
        && (c.contains("limited") || c.contains("hurry") || c.contains("last chance") || c.contains("eligible"))
    {
        return true;
    }
    // Classic: discord invites paired with bait, or mention floods.
    if c.contains("discord.gg/")
        && (c.contains("airdrop") || c.contains("free") || c.contains("claim"))
    {
        return true;
    }
    let mentions =
        c.matches("<@").count() + c.matches("@everyone").count() + c.matches("@here").count();
    mentions >= 5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_homoglyph_ion_wave() {
        // Real wave text (Cyrillic lookalikes).
        assert!(is_spam("🎁Osmоsis hаs lаunchеd its sеcоnd IОN Аirdrоp!🎁"));
        assert!(is_spam("🎁 Osmosis has launched its second ION Airdrop! Claim now 🎁"));
        assert!(is_spam("Osmosis second ION airdrop launched, claim your tokens"));
    }

    #[test]
    fn no_false_positives_on_legit_chat() {
        assert!(!is_spam("has anyone tried the new osmosis frontend?"));
        assert!(!is_spam("ion propulsion is cool, unrelated"));
        assert!(!is_spam("gm everyone, validator update posted in #announcements"));
        assert!(!is_spam("claim your rewards from staking each epoch"));
    }
}
