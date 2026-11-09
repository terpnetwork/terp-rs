//! NIP-21 — nostr: URI scheme.
//!
//! Standard URI scheme for Nostr entities.
//! URIs follow: `nostr:<NIP-19 bech32 string>`
//! Supports nprofile, nevent, naddr, npub (NOT nsec per spec).
//!
//! Also provides HTML `<link>` tag generation for webpage↔Nostr entity association.
//!
//! Spec: https://nips.nostr.com/21

use crate::error::NipResult;
use crate::nips::nip19::Nip19Entity;

/// A `nostr:` URI wrapping a NIP-19 entity.
///
/// Automatically excludes nsec entities (not allowed per NIP-21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrUri(Nip19Entity);

impl NostrUri {
    /// Create a `nostr:` URI from any NIP-19 entity.
    ///
    /// Returns an error if the entity is an nsec (private key),
    /// which is explicitly excluded from NIP-21.
    pub fn new(entity: Nip19Entity) -> Result<Self, &'static str> {
        if matches!(entity, Nip19Entity::Nsec(_)) {
            return Err("nsec private keys are not allowed in nostr: URIs per NIP-21");
        }
        Ok(Self(entity))
    }

    /// Parse a `nostr:` URI string into a NostrUri.
    ///
    /// Strips the `nostr:` prefix and decodes the bech32 body.
    pub fn parse(uri: &str) -> NipResult<Self> {
        let body = uri
            .strip_prefix("nostr:")
            .ok_or_else(|| crate::error::NipError::Validation("missing nostr: prefix".into()))?;
        let entity = Nip19Entity::decode(body)?;
        Self::new(entity).map_err(|e| crate::error::NipError::Validation(e.into()))
    }

    /// Get the inner NIP-19 entity.
    pub fn entity(&self) -> &Nip19Entity {
        &self.0
    }

    /// Consume into the inner NIP-19 entity.
    pub fn into_inner(self) -> Nip19Entity {
        self.0
    }

    /// Get the bech32 string (without `nostr:` prefix).
    pub fn bech32(&self) -> String {
        self.0.encode()
    }

    /// Get the full URI string.
    pub fn as_uri(&self) -> String {
        format!("nostr:{}", self.0.encode())
    }

    /// Generate an HTML `<link rel="alternate">` tag associating a webpage
    /// with a Nostr event (naddr).
    ///
    /// Example:
    /// ```html
    /// <link rel="alternate" href="nostr:naddr1..." />
    /// ```
    pub fn to_alternate_link_tag(&self) -> String {
        format!(r#"  <link rel="alternate" href="{}" />"#, self.as_uri())
    }

    /// Generate an HTML `<link rel="me">` or `<link rel="author">` tag
    /// associating authorship of a webpage with a Nostr profile (nprofile or npub).
    ///
    /// # Errors
    ///
    /// Returns an error if the entity is not a profile type (nprofile or npub).
    pub fn to_profile_link_tag(&self, rel: &str) -> Result<String, &'static str> {
        match &self.0 {
            Nip19Entity::Npub(_) | Nip19Entity::Nprofile { .. } => Ok(format!(
                r#"  <link rel="{}" href="{}" />"#,
                rel,
                self.as_uri()
            )),
            _ => Err(
                "can only link profile entities (npub/nprofile) with rel=\"me\" or rel=\"author\"",
            ),
        }
    }

    /// Validate the entity according to NIP-21 rules.
    pub fn validate(&self) -> Result<(), &'static str> {
        match &self.0 {
            Nip19Entity::Nsec(_) => Err("nsec not allowed in nostr: URIs"),
            _ => Ok(()),
        }
    }
}

impl std::fmt::Display for NostrUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_uri())
    }
}

impl TryFrom<Nip19Entity> for NostrUri {
    type Error = &'static str;

    fn try_from(entity: Nip19Entity) -> Result<Self, Self::Error> {
        Self::new(entity)
    }
}

impl TryFrom<&str> for NostrUri {
    type Error = crate::error::NipError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nips::nip19;

    #[test]
    fn test_npub_uri_roundtrip() {
        let entity = nip19::npub_from_hex(
            "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d",
        )
        .unwrap();
        let uri = NostrUri::new(entity.clone()).unwrap();
        let uri_str = uri.as_uri();
        assert!(uri_str.starts_with("nostr:npub1"));

        let parsed = NostrUri::parse(&uri_str).unwrap();
        assert_eq!(parsed.entity(), &entity);
    }

    #[test]
    fn test_nprofile_uri_roundtrip() {
        let entity = Nip19Entity::Nprofile {
            pubkey: [0xabu8; 32],
            relays: vec!["wss://relay.example.com".into()],
        };
        let uri = NostrUri::new(entity.clone()).unwrap();
        let parsed = NostrUri::parse(&uri.to_string()).unwrap();
        assert_eq!(parsed.entity(), &entity);
    }

    #[test]
    fn test_nsec_rejected() {
        let entity = Nip19Entity::Nsec([0xabu8; 32]);
        let result = NostrUri::new(entity);
        assert!(result.is_err());
    }

    #[test]
    fn test_nevent_uri() {
        let entity = Nip19Entity::Nevent {
            event_id: [0x42u8; 32],
            relays: vec![],
            author: None,
            kind: None,
        };
        let uri = NostrUri::new(entity).unwrap();
        assert!(uri.as_uri().starts_with("nostr:nevent1"));
    }

    #[test]
    fn test_naddr_uri() {
        let entity = Nip19Entity::Naddr {
            identifier: "my-article".into(),
            pubkey: [0xabu8; 32],
            kind: 30023,
            relays: vec![],
        };
        let uri = NostrUri::new(entity).unwrap();
        assert!(uri.as_uri().starts_with("nostr:naddr1"));
    }

    #[test]
    fn test_invalid_prefix() {
        let result = NostrUri::parse("nostrinvalid:npub1...");
        assert!(result.is_err());
    }

    #[test]
    fn test_alternate_link_tag() {
        let entity = Nip19Entity::Naddr {
            identifier: "test".into(),
            pubkey: [0xabu8; 32],
            kind: 30023,
            relays: vec![],
        };
        let uri = NostrUri::new(entity).unwrap();
        let tag = uri.to_alternate_link_tag();
        assert!(tag.contains("rel=\"alternate\""));
        assert!(tag.contains("nostr:naddr1"));
    }

    #[test]
    fn test_profile_link_tag() {
        let entity = Nip19Entity::Npub([0xabu8; 32]);
        let uri = NostrUri::new(entity).unwrap();
        let tag = uri.to_profile_link_tag("me").unwrap();
        assert!(tag.contains("rel=\"me\""));
        assert!(tag.contains("nostr:npub1"));
    }

    #[test]
    fn test_nevent_profile_link_rejected() {
        let entity = Nip19Entity::Nevent {
            event_id: [0x42u8; 32],
            relays: vec![],
            author: None,
            kind: None,
        };
        let uri = NostrUri::new(entity).unwrap();
        let result = uri.to_profile_link_tag("me");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate() {
        let entity = Nip19Entity::Npub([0xabu8; 32]);
        let uri = NostrUri::new(entity).unwrap();
        assert!(uri.validate().is_ok());
    }
}
