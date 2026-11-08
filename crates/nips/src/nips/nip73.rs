//! NIP-73: External Content IDs
//! ## NIP-73 External Content IDs
//!
//! ```rust
//! use cw721_nips::Tag;
//! use cw721_nips::nips::nip73::ExternalId;
//!
//! // `NostrEventBuilder::build` appends these tags for every id returned by
//! // `Nip73Ext::external_ids` (blanket-implemented for all `NipMetadata`).
//! let ext = ExternalId::new("isbn:9780765382030", "isbn");
//! let (i_tag, k_tag) = Tag::i(ext.id, ext.kind);
//! assert_eq!(i_tag.0, vec!["i", "isbn:9780765382030"]);
//! assert_eq!(k_tag.0, vec!["k", "isbn"]);
//! ```
//!
//! See [`Nip73Ext`] and [`ExternalId`].

use crate::Tag;

/// NIP-73: External Content ID support for any metadata
pub trait Nip73Ext: crate::NipMetadata {
    /// Returns list of external content IDs this metadata references.
    /// Override this in your NIP types.
    fn external_ids(&self) -> Vec<ExternalId> {
        vec![]
    }

    /// Add NIP-73 tags to existing tags
    fn with_external_ids_tags(&self, mut tags: Vec<Tag>) -> Vec<Tag> {
        for ext in self.external_ids() {
            let (i_tag, k_tag) = Tag::i(ext.id, ext.kind);
            tags.push(i_tag);
            tags.push(k_tag);

            if let Some(hint) = ext.url_hint {
                tags.push(Tag(vec!["i".to_string(), hint]));
            }
        }
        tags
    }
}

impl<T: crate::NipMetadata> Nip73Ext for T {}

/// External Content ID (NIP-73)
#[derive(Debug, Clone)]
pub struct ExternalId {
    pub id: String,
    pub kind: String,
    pub url_hint: Option<String>,
}

impl ExternalId {
    pub fn new(id: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            url_hint: None,
        }
    }

    pub fn with_hint(mut self, url: impl Into<String>) -> Self {
        self.url_hint = Some(url.into());
        self
    }
}
