//! # cw721-nostr-extensions
//!
//! Bridge between Nostr NIP metadata and cw721 NFT extensions.
//! Provides binary-encoded, schema-agnostic metadata storage with blanket trait implementations.

use cosmwasm_std::{Binary, StdError, StdResult, to_json_binary};
use cw721::Attribute;
use cw721::error::Cw721ContractError;
use cw721::traits::{
    Contains, Cw721CustomMsg, Cw721State, FromAttributesState, StateFactory, ToAttributesState,
};
use schemars::JsonSchema;
use schemars::r#gen::SchemaGenerator;
use schemars::schema::{InstanceType, ObjectValidation, Schema, SchemaObject};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use std::marker::PhantomData;

use crate::{NipKind, NipMetadata, RawNostrEvent, nips};

/// Core trait for Nostr-compatible cw721 extensions.
///
/// Automatically implemented for all `NipMetadata` types via blanket impl.
pub trait NostrCw721Ext: NipMetadata {
    /// Encode to binary for on-chain storage
    fn to_storage_binary(&self) -> StdResult<Binary>;

    /// Decode from binary
    fn from_storage_binary(binary: &Binary) -> Result<Self, Cw721ContractError>;

    /// Convert to cw721 attributes (kind discriminator + binary data)
    fn to_attributes(&self) -> Result<Vec<Attribute>, Cw721ContractError>;

    /// Parse from cw721 attributes
    fn from_attributes(attrs: &[Attribute]) -> Result<Self, Cw721ContractError>;

    /// Validate for cw721 context (NIP validation + blockchain-specific rules)
    fn validate_cw721(&self, block_time: u64) -> Result<(), Cw721ContractError>;
}

/// Blanket implementation for all NipMetadata types.
///
/// Uses JSON for binary encoding and stores [kind, data] as cw721 `Attribute` pairs.
impl<T: NipMetadata> NostrCw721Ext for T {
    fn to_storage_binary(&self) -> StdResult<Binary> {
        Ok(Binary::from(serde_json::to_vec(self)?))
    }

    fn from_storage_binary(binary: &Binary) -> Result<Self, Cw721ContractError> {
        serde_json::from_slice(binary.as_slice()).map_err(|e| {
            Cw721ContractError::Std(StdError::msg(format!(
                "Failed to decode {}: {}",
                std::any::type_name::<T>(),
                e
            )))
        })
    }

    fn to_attributes(&self) -> Result<Vec<Attribute>, Cw721ContractError> {
        let json = self
            .to_storage_binary()
            .map_err(|e| Cw721ContractError::Std(e))?;

        Ok(vec![
            Attribute {
                key: "nip_kind".to_string(),
                value: to_json_binary(&self.kind().kind_value())
                    .map_err(|e| Cw721ContractError::Std(e))?,
            },
            Attribute {
                key: "nip_data".to_string(),
                value: json,
            },
        ])
    }

    fn from_attributes(attrs: &[Attribute]) -> Result<Self, Cw721ContractError> {
        let data_attr = attrs
            .iter()
            .find(|a| a.key == "nip_data")
            .ok_or_else(|| Cw721ContractError::Std(StdError::msg("Missing nip_data attribute")))?;

        Self::from_storage_binary(&data_attr.value)
    }

    fn validate_cw721(&self, block_time: u64) -> Result<(), Cw721ContractError> {
        // NIP-specific validation
        self.validate().map_err(|e| {
            Cw721ContractError::Std(StdError::msg(format!("NIP validation failed: {:?}", e)))
        })?;

        // Override in specific impls for cw721-specific rules
        let _ = block_time;
        Ok(())
    }
}

// ============================================================================
// Universal Nostr metadata extension for cw721
// ============================================================================

/// Universal Nostr metadata extension.
///
/// This is the concrete type used in `Cw721Extensions<..., MetadataExt, ...>`.
/// It stores any NIP metadata type in binary form with a kind discriminator.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct MetadataExt {
    /// NIP kind (31922, 31923, 31924, 31925, etc.)
    pub kind: u16,
    /// Binary-encoded NIP metadata
    pub data: Binary,
    /// Optional: original Nostr event ID for verification
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// Optional: author pubkey
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Indexed fields for querying (populated by specific NIPs)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub indexed: Vec<Attribute>,
}

impl MetadataExt {
    /// Create from any NipMetadata type
    pub fn new<T: NostrCw721Ext>(metadata: &T) -> StdResult<Self> {
        Ok(Self {
            kind: metadata.kind().kind_value(),
            data: metadata.to_storage_binary()?,
            event_id: None,
            author: None,
            indexed: Vec::new(),
        })
    }

    /// Create with event tracking
    pub fn with_event<T: NostrCw721Ext>(
        metadata: &T,
        event_id: String,
        author: String,
    ) -> StdResult<Self> {
        Ok(Self {
            kind: metadata.kind().kind_value(),
            data: metadata.to_storage_binary()?,
            event_id: Some(event_id),
            author: Some(author),
            indexed: Vec::new(),
        })
    }

    /// Decode to a specific NIP type.
    ///
    /// For types with a stable kind (e.g. `CalendarMetadata` at kind 31924),
    /// use [`decode_static`] for compile-time kind checking.
    pub fn decode<T: NostrCw721Ext>(&self) -> Result<T, Cw721ContractError> {
        T::from_storage_binary(&self.data)
    }

    /// Decode to a type that has a static kind, verifying the stored kind matches.
    pub fn decode_static<T: NostrCw721Ext + StaticNipKind>(&self) -> Result<T, Cw721ContractError> {
        let expected = T::KIND;
        if self.kind != expected {
            return Err(Cw721ContractError::Std(StdError::msg(format!(
                "Kind mismatch: expected {}, got {}",
                expected, self.kind
            ))));
        }
        T::from_storage_binary(&self.data)
    }

    /// Add an indexed field for querying.
    /// Values are JSON-encoded automatically.
    pub fn with_index(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> StdResult<Self> {
        self.indexed.push(Attribute {
            key: key.into(),
            value: to_json_binary(&value.into())?,
        });
        Ok(self)
    }

    /// Get an indexed value as a string (decoded from JSON binary).
    pub fn get_index(&self, key: &str) -> Option<String> {
        self.indexed
            .iter()
            .find(|a| a.key == key)
            .and_then(|a| a.value::<String>().ok())
    }
}

/// Trait for types that have a static kind value.
/// Used for type-safe decoding.
pub trait StaticNipKind: NostrCw721Ext {
    const KIND: u16;

    fn kind_value_static() -> u16 {
        Self::KIND
    }
}

impl StaticNipKind for nips::nip52::CalendarEventMetadata {
    // CalendarEventMetadata can be 31922 (date-based) or 31923 (time-based).
    // Use decode_unchecked() for variable-kind types.
    const KIND: u16 = 0; // placeholder — use decode() not decode_static()
}

impl StaticNipKind for nips::nip52::CalendarMetadata {
    const KIND: u16 = 31924;
}

impl StaticNipKind for nips::nip52::RSVPMetadata {
    const KIND: u16 = 31925;
}

// ============================================================================
// Type-safe wrapper for specific NIP metadata
// ============================================================================

/// Type-safe wrapper for a single known NIP kind.
///
/// Use this when the contract only works with one NIP kind at compile time.
#[derive(Serialize, Deserialize, Clone)]
pub struct TypedMetadataExt<T: NostrCw721Ext> {
    pub data: Binary,
    pub event_id: Option<String>,
    pub author: Option<String>,
    #[serde(skip)]
    pub _phantom: PhantomData<T>,
}

// Manual implementations to avoid trait bounds on generic T leaking into derives.

impl<T: NostrCw721Ext> std::fmt::Debug for TypedMetadataExt<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedMetadataExt")
            .field("data", &self.data)
            .field("event_id", &self.event_id)
            .field("author", &self.author)
            .finish()
    }
}

impl<T: NostrCw721Ext> PartialEq for TypedMetadataExt<T> {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data && self.event_id == other.event_id && self.author == other.author
    }
}

impl<T: NostrCw721Ext> JsonSchema for TypedMetadataExt<T> {
    fn schema_name() -> String {
        "TypedMetadataExt".to_string()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(InstanceType::Object.into()),
            object: Some(Box::new(ObjectValidation {
                properties: BTreeMap::from([
                    (
                        "data".to_string(),
                        Schema::Object(SchemaObject {
                            instance_type: Some(InstanceType::String.into()),
                            format: Some("binary".to_string()),
                            ..Default::default()
                        }),
                    ),
                    (
                        "event_id".to_string(),
                        Schema::Object(SchemaObject {
                            instance_type: Some(
                                vec![InstanceType::String, InstanceType::Null].into(),
                            ),
                            ..Default::default()
                        }),
                    ),
                    (
                        "author".to_string(),
                        Schema::Object(SchemaObject {
                            instance_type: Some(
                                vec![InstanceType::String, InstanceType::Null].into(),
                            ),
                            ..Default::default()
                        }),
                    ),
                ]),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

impl<T: NostrCw721Ext> TypedMetadataExt<T> {
    pub fn new(metadata: &T) -> StdResult<Self> {
        Ok(Self {
            data: metadata.to_storage_binary()?,
            event_id: None,
            author: None,
            _phantom: PhantomData,
        })
    }

    pub fn with_event(metadata: &T, event_id: String, author: String) -> StdResult<Self> {
        Ok(Self {
            data: metadata.to_storage_binary()?,
            event_id: Some(event_id),
            author: Some(author),
            _phantom: PhantomData,
        })
    }

    pub fn decode(&self) -> Result<T, Cw721ContractError> {
        T::from_storage_binary(&self.data)
    }
}

// ============================================================================
// CW721 Trait Implementations
// ============================================================================

impl Cw721State for MetadataExt {}
impl Cw721CustomMsg for MetadataExt {}

impl Contains for MetadataExt {
    fn contains(&self, other: &Self) -> bool {
        if self.kind != other.kind {
            return false;
        }

        match self.kind {
            31922 | 31923 => {
                let self_decoded: Result<nips::nip52::CalendarEventMetadata, _> =
                    serde_json::from_slice(self.data.as_slice());
                let other_decoded: Result<nips::nip52::CalendarEventMetadata, _> =
                    serde_json::from_slice(other.data.as_slice());

                match (self_decoded, other_decoded) {
                    (Ok(s), Ok(o)) => {
                        // Self contains other if same identifier and wider time range
                        s.d_tag == o.d_tag
                            && s.start_time <= o.start_time
                            && s.end_time >= o.end_time
                    }
                    _ => false,
                }
            }
            _ => self.data == other.data,
        }
    }
}

impl ToAttributesState for MetadataExt {
    fn to_attributes_state(&self) -> Result<Vec<Attribute>, Cw721ContractError> {
        let mut attrs = vec![
            Attribute {
                key: "nip_kind".to_string(),
                value: to_json_binary(&self.kind).map_err(|e| Cw721ContractError::Std(e))?,
            },
            Attribute {
                key: "nip_data".to_string(),
                value: self.data.clone(),
            },
        ];

        if let Some(ref id) = self.event_id {
            attrs.push(Attribute {
                key: "nip_event_id".to_string(),
                value: to_json_binary(id).map_err(|e| Cw721ContractError::Std(e))?,
            });
        }

        if let Some(ref author) = self.author {
            attrs.push(Attribute {
                key: "nip_author".to_string(),
                value: to_json_binary(author).map_err(|e| Cw721ContractError::Std(e))?,
            });
        }

        for idx in &self.indexed {
            attrs.push(Attribute {
                key: format!("nip_idx_{}", idx.key),
                value: idx.value.clone(),
            });
        }

        Ok(attrs)
    }
}

impl FromAttributesState for MetadataExt {
    fn from_attributes_state(attrs: &[Attribute]) -> Result<Self, Cw721ContractError> {
        let mut kind = 0u16;
        let mut data = Binary::default();
        let mut event_id: Option<String> = None;
        let mut author: Option<String> = None;
        let mut indexed = Vec::new();

        for attr in attrs {
            match attr.key.as_str() {
                "nip_kind" => {
                    kind = attr.value::<u16>()?;
                }
                "nip_data" => {
                    data = attr.value.clone();
                }
                "nip_event_id" => {
                    event_id = Some(attr.value::<String>()?);
                }
                "nip_author" => {
                    author = Some(attr.value::<String>()?);
                }
                key if key.starts_with("nip_idx_") => {
                    indexed.push(Attribute {
                        key: key.strip_prefix("nip_idx_").unwrap_or(key).to_string(),
                        value: attr.value.clone(),
                    });
                }
                _ => {}
            }
        }

        if kind == 0 || data.is_empty() {
            return Err(Cw721ContractError::Std(StdError::msg(
                "Missing required NIP fields",
            )));
        }

        Ok(Self {
            kind,
            data,
            event_id,
            author,
            indexed,
        })
    }
}

impl StateFactory<MetadataExt> for MetadataExt {
    fn create(
        &self,
        _deps: cosmwasm_std::Deps,
        _env: &cosmwasm_std::Env,
        _info: Option<&cosmwasm_std::MessageInfo>,
        _current: Option<&MetadataExt>,
    ) -> Result<MetadataExt, Cw721ContractError> {
        self.validate_by_kind()?;
        Ok(self.clone())
    }

    fn validate(
        &self,
        _deps: cosmwasm_std::Deps,
        env: &cosmwasm_std::Env,
        _info: Option<&cosmwasm_std::MessageInfo>,
        _current: Option<&MetadataExt>,
    ) -> Result<(), Cw721ContractError> {
        let block_time = env.block.time.seconds();

        match self.kind {
            31922 | 31923 => {
                let event: nips::nip52::CalendarEventMetadata = self.decode()?;
                event.validate_cw721(block_time)?;

                if event.end_time <= event.start_time {
                    return Err(Cw721ContractError::Std(StdError::msg(
                        "Event end time must be after start time",
                    )));
                }
            }
            31924 => {
                let calendar: nips::nip52::CalendarMetadata = self.decode()?;
                calendar.validate_cw721(block_time)?;
            }
            31925 => {
                let rsvp: nips::nip52::RSVPMetadata = self.decode()?;
                rsvp.validate_cw721(block_time)?;
            }
            _ => {
                // Unknown kind — verify it's valid JSON
                serde_json::from_slice::<serde_json::Value>(self.data.as_slice()).map_err(|e| {
                    Cw721ContractError::Std(StdError::msg(format!("Invalid JSON data: {}", e)))
                })?;
            }
        }

        Ok(())
    }
}

impl MetadataExt {
    /// Validate by attempting to decode based on known kind.
    fn validate_by_kind(&self) -> Result<(), Cw721ContractError> {
        match self.kind {
            31922 | 31923 => {
                let _: nips::nip52::CalendarEventMetadata = self.decode()?;
            }
            31924 => {
                let _: nips::nip52::CalendarMetadata = self.decode()?;
            }
            31925 => {
                let _: nips::nip52::RSVPMetadata = self.decode()?;
            }
            _ => {}
        }
        Ok(())
    }
}

// ============================================================================
// Generic implementations for TypedMetadataExt<T>
// ============================================================================

impl<T: NostrCw721Ext> Cw721State for TypedMetadataExt<T> {}
impl<T: NostrCw721Ext> Cw721CustomMsg for TypedMetadataExt<T> {}

impl<T: NostrCw721Ext + PartialEq> Contains for TypedMetadataExt<T> {
    fn contains(&self, other: &Self) -> bool {
        match (self.decode(), other.decode()) {
            (Ok(s), Ok(o)) => s == o,
            _ => false,
        }
    }
}

impl<T: NostrCw721Ext> ToAttributesState for TypedMetadataExt<T> {
    fn to_attributes_state(&self) -> Result<Vec<Attribute>, Cw721ContractError> {
        let mut attrs = vec![Attribute {
            key: "nip_data".to_string(),
            value: self.data.clone(),
        }];

        if let Some(ref id) = self.event_id {
            attrs.push(Attribute {
                key: "nip_event_id".to_string(),
                value: to_json_binary(id).map_err(|e| Cw721ContractError::Std(e))?,
            });
        }

        if let Some(ref author) = self.author {
            attrs.push(Attribute {
                key: "nip_author".to_string(),
                value: to_json_binary(author).map_err(|e| Cw721ContractError::Std(e))?,
            });
        }

        Ok(attrs)
    }
}

impl<T: NostrCw721Ext> FromAttributesState for TypedMetadataExt<T> {
    fn from_attributes_state(attrs: &[Attribute]) -> Result<Self, Cw721ContractError> {
        let mut data = Binary::default();
        let mut event_id: Option<String> = None;
        let mut author: Option<String> = None;

        for attr in attrs {
            match attr.key.as_str() {
                "nip_data" => data = attr.value.clone(),
                "nip_event_id" => event_id = Some(attr.value::<String>()?),
                "nip_author" => author = Some(attr.value::<String>()?),
                _ => {}
            }
        }

        if data.is_empty() {
            return Err(Cw721ContractError::Std(StdError::msg("Missing nip_data")));
        }

        // Validate it decodes correctly
        let _: T = T::from_storage_binary(&data)?;

        Ok(Self {
            data,
            event_id,
            author,
            _phantom: PhantomData,
        })
    }
}

impl<T: NostrCw721Ext> StateFactory<TypedMetadataExt<T>> for TypedMetadataExt<T> {
    fn create(
        &self,
        _deps: cosmwasm_std::Deps,
        _env: &cosmwasm_std::Env,
        _info: Option<&cosmwasm_std::MessageInfo>,
        _current: Option<&TypedMetadataExt<T>>,
    ) -> Result<TypedMetadataExt<T>, Cw721ContractError> {
        let _: T = T::from_storage_binary(&self.data)?;
        Ok(self.clone())
    }

    fn validate(
        &self,
        _deps: cosmwasm_std::Deps,
        env: &cosmwasm_std::Env,
        _info: Option<&cosmwasm_std::MessageInfo>,
        _current: Option<&TypedMetadataExt<T>>,
    ) -> Result<(), Cw721ContractError> {
        let decoded: T = T::from_storage_binary(&self.data)?;
        decoded.validate_cw721(env.block.time.seconds())
    }
}

// ============================================================================
// Helper functions and builders
// ============================================================================

/// Builder for creating MetadataExt with indexed fields.
pub struct MetadataExtBuilder {
    kind: u16,
    data: Binary,
    event_id: Option<String>,
    author: Option<String>,
    indexed: Vec<Attribute>,
}

impl MetadataExtBuilder {
    pub fn new<T: NostrCw721Ext>(metadata: &T) -> StdResult<Self> {
        Ok(Self {
            kind: metadata.kind().kind_value(),
            data: metadata.to_storage_binary()?,
            event_id: None,
            author: None,
            indexed: Vec::new(),
        })
    }

    pub fn event_id(mut self, id: impl Into<String>) -> Self {
        self.event_id = Some(id.into());
        self
    }

    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    pub fn index(mut self, key: impl Into<String>, value: impl Into<String>) -> StdResult<Self> {
        self.indexed.push(Attribute {
            key: key.into(),
            value: to_json_binary(&value.into())?,
        });
        Ok(self)
    }

    pub fn build(self) -> MetadataExt {
        MetadataExt {
            kind: self.kind,
            data: self.data,
            event_id: self.event_id,
            author: self.author,
            indexed: self.indexed,
        }
    }
}

// ============================================================================
// Convenience re-exports for NIP-52
// ============================================================================

pub mod nip52 {
    use super::*;
    use crate::nips::nip52::{CalendarEventMetadata, CalendarMetadata, RSVPMetadata};

    /// Type-safe calendar event extension (kind 31922 or 31923).
    pub type CalendarEventExt = TypedMetadataExt<CalendarEventMetadata>;

    /// Type-safe calendar extension (kind 31924).
    pub type CalendarExt = TypedMetadataExt<CalendarMetadata>;

    /// Type-safe RSVP extension (kind 31925).
    pub type RSVPExt = TypedMetadataExt<RSVPMetadata>;

    /// Extension trait for calendar-specific functionality on MetadataExt.
    pub trait CalendarEventExtHelpers {
        fn is_time_based(&self) -> bool;
        fn is_date_based(&self) -> bool;
        fn duration_seconds(&self) -> u64;
    }

    impl CalendarEventExtHelpers for MetadataExt {
        fn is_time_based(&self) -> bool {
            self.kind == 31923
        }

        fn is_date_based(&self) -> bool {
            self.kind == 31922
        }

        fn duration_seconds(&self) -> u64 {
            if let Ok(event) = self.decode::<CalendarEventMetadata>() {
                event.end_time.saturating_sub(event.start_time)
            } else {
                0
            }
        }
    }
}

/// Builder trait for constructing on-chain or off-chain cw721
/// Nostr metadata extensions.
///
/// Every `NipMetadata` type gets a blanket impl that produces the
/// universal [`MetadataExt`]. Concrete types can override to return
/// a contract-specific extension type.
pub trait NostrCw721Builder {
    /// The output extension type.
    type Extension: Serialize + DeserializeOwned + Clone;

    /// Build on-chain metadata from a full Nostr event.
    fn onchain_metadata(event: &RawNostrEvent) -> StdResult<Self::Extension>;

    /// Build off-chain metadata with a CID pointer.
    fn offchain_metadata(cid: String, kind: u16) -> StdResult<Self::Extension>;
}

/// Concrete impl for [`MetadataExt`] itself: constructs `Self` directly.
impl NostrCw721Builder for MetadataExt {
    type Extension = Self;

    fn onchain_metadata(event: &RawNostrEvent) -> StdResult<Self> {
        Ok(Self {
            kind: event.kind,
            data: to_json_binary(event)?,
            event_id: Some(event.id.clone()),
            author: Some(event.pubkey.clone()),
            indexed: vec![],
        })
    }

    fn offchain_metadata(cid: String, kind: u16) -> StdResult<Self> {
        Ok(Self {
            kind,
            data: to_json_binary(&cid)?,
            event_id: None,
            author: None,
            indexed: vec![],
        })
    }
}
