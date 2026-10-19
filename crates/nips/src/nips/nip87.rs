//! NIP-87: Ecash Mint Discoverability
//!
//! <https://github.com/nostr-protocol/nips/blob/master/87.md>

use crate::{
    NipKind, NipMetadata, RawNostrEvent, Tag,
    error::{NipError, NipResult},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ==================== NIP-87 Kinds ====================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Nip87Kind {
    Recommendation,
    CashuMint,
    Fedimint,
}

impl NipKind for Nip87Kind {
    fn kind_value(&self) -> u16 {
        match self {
            Nip87Kind::Recommendation => 38000,
            Nip87Kind::CashuMint => 38172,
            Nip87Kind::Fedimint => 38173,
        }
    }
}

// ==================== Common Types ====================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct MintReference {
    pub kind: u16,
    pub pubkey: String,
    pub d_identifier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
}

impl MintReference {
    pub fn new(kind: u16, pubkey: impl Into<String>, d_identifier: impl Into<String>) -> Self {
        Self {
            kind,
            pubkey: pubkey.into(),
            d_identifier: d_identifier.into(),
            relay_url: None,
        }
    }

    pub fn with_relay(mut self, relay: impl Into<String>) -> Self {
        self.relay_url = Some(relay.into());
        self
    }

    pub fn to_a_tag(&self) -> Tag {
        if let Some(ref relay) = self.relay_url {
            Tag::a_with_relay(self.kind, &self.pubkey, &self.d_identifier, relay)
        } else {
            Tag::a(self.kind, &self.pubkey, &self.d_identifier)
        }
    }
}

// ==================== Mint Recommendation (kind:38000) ====================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct MintRecommendation {
    pub d_tag: String,
    pub recommended_kind: u16,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub u_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub a_tags: Vec<MintReference>,
}

impl MintRecommendation {
    pub fn new(
        d_tag: impl Into<String>,
        recommended_kind: u16,
        content: impl Into<String>,
    ) -> Self {
        Self {
            d_tag: d_tag.into(),
            recommended_kind,
            content: content.into(),
            u_tags: vec![],
            a_tags: vec![],
        }
    }

    pub fn with_u(mut self, url_or_invite: impl Into<String>) -> Self {
        self.u_tags.push(url_or_invite.into());
        self
    }

    pub fn with_a(mut self, reference: MintReference) -> Self {
        self.a_tags.push(reference);
        self
    }
}

impl NipMetadata for MintRecommendation {
    type Kind = Nip87Kind;

    fn kind(&self) -> Self::Kind {
        Nip87Kind::Recommendation
    }

    fn validate(&self) -> NipResult<()> {
        if self.d_tag.is_empty() {
            return Err(NipError::Nip87("d_tag cannot be empty".into()));
        }
        if self.recommended_kind != 38172 && self.recommended_kind != 38173 {
            return Err(NipError::Nip87(
                "recommended_kind must be 38172 or 38173".into(),
            ));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        let mut tags = vec![
            Tag::d(&self.d_tag),
            Tag::k(self.recommended_kind), // We'll add this method
        ];

        for u in &self.u_tags {
            tags.push(Tag::new(vec!["u".to_string(), u.clone()]));
        }

        for a in &self.a_tags {
            tags.push(a.to_a_tag());
        }

        tags
    }

    fn content(&self) -> String {
        self.content.clone()
    }

    fn d_tag(&self) -> Option<String> {
        Some(self.d_tag.clone())
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != 38000 {
            return Err(NipError::Nip87(format!(
                "Expected kind 38000, got {}",
                event.kind
            )));
        }

        let d_tag = event
            .d_tag()
            .ok_or_else(|| NipError::Nip87("Missing d tag".into()))?
            .to_string();

        let mut recommended_kind = 0u16;
        let mut u_tags = vec![];
        let mut a_tags = vec![];

        for tag in &event.tags {
            if tag.is_empty() {
                continue;
            }
            match tag[0].as_str() {
                "k" => {
                    if let Some(v) = tag.get(1) {
                        recommended_kind = v
                            .parse()
                            .map_err(|_| NipError::Nip87("Invalid k tag value".into()))?;
                    }
                }
                "u" => {
                    if let Some(v) = tag.get(1) {
                        u_tags.push(v.clone());
                    }
                }
                "a" => {
                    // Simplified parsing - can be expanded
                    if tag.len() >= 2 {
                        let parts: Vec<&str> = tag[1].split(':').collect();
                        if parts.len() >= 3 {
                            if let Ok(kind) = parts[0].parse() {
                                a_tags.push(MintReference::new(kind, parts[1], parts[2]));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(Self {
            d_tag,
            recommended_kind,
            content: event.content.clone(),
            u_tags,
            a_tags,
        })
    }
}

// ==================== Cashu Mint (kind:38172) & Fedimint (kind:38173) ====================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct CashuMintMetadata {
    pub d_tag: String,
    pub content: String,
    pub u_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nuts: Vec<String>,
    pub network: String,
}

impl CashuMintMetadata {
    pub fn new(
        d_tag: impl Into<String>,
        u_url: impl Into<String>,
        network: impl Into<String>,
    ) -> Self {
        Self {
            d_tag: d_tag.into(),
            content: String::new(),
            u_url: u_url.into(),
            nuts: vec![],
            network: network.into(),
        }
    }

    pub fn with_nuts(mut self, nuts: Vec<impl Into<String>>) -> Self {
        self.nuts = nuts.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = content.into();
        self
    }
}

impl NipMetadata for CashuMintMetadata {
    type Kind = Nip87Kind;

    fn kind(&self) -> Self::Kind {
        Nip87Kind::CashuMint
    }

    fn validate(&self) -> NipResult<()> {
        if self.d_tag.is_empty() || self.u_url.is_empty() {
            return Err(NipError::Nip87(
                "d_tag and u_url are required for Cashu mint".into(),
            ));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        let mut tags = vec![
            Tag::d(&self.d_tag),
            Tag::new(vec!["u".to_string(), self.u_url.clone()]),
            Tag::new(vec!["n".to_string(), self.network.clone()]),
        ];

        if !self.nuts.is_empty() {
            tags.push(Tag::new(vec!["nuts".to_string(), self.nuts.join(",")]));
        }

        tags
    }

    fn content(&self) -> String {
        self.content.clone()
    }

    fn d_tag(&self) -> Option<String> {
        Some(self.d_tag.clone())
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != 38172 {
            return Err(NipError::Nip87(format!(
                "Expected kind 38172, got {}",
                event.kind
            )));
        }
        // Full parsing similar to NIP-52 (left as exercise or expand later)
        Err(NipError::Nip87(
            "from_raw_event not fully implemented yet".into(),
        ))
    }
}

// Similar struct for FedimintMetadata...

// ==================== NIP-44 Encryption Support ====================
#[cfg(feature = "nip44")]
impl crate::nips::nip44::NipMetadataEncrypt for MintRecommendation {
    fn encrypted_content(&self) -> &str {
        &self.content
    }

    fn set_encrypted_content(&mut self, payload: String) {
        self.content = payload;
    }
}
#[cfg(feature = "nip44")]
impl crate::nips::nip44::NipMetadataEncrypt for CashuMintMetadata {
    fn encrypted_content(&self) -> &str {
        &self.content
    }

    fn set_encrypted_content(&mut self, payload: String) {
        self.content = payload;
    }
}
