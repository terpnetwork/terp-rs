use crate::{
    NipKind, NipMetadata, RawNostrEvent, Tag,
    error::{NipError, NipResult},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ==================== NIP-15 Kinds ====================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Nip15Kind {
    SetStall,
    SetProduct,
    MarketplaceUI,
    AuctionProduct,
    Bid,
    BidConfirmation,
}

impl NipKind for Nip15Kind {
    fn kind_value(&self) -> u16 {
        match self {
            Nip15Kind::SetStall => 30017,
            Nip15Kind::SetProduct => 30018,
            Nip15Kind::MarketplaceUI => 30019,
            Nip15Kind::AuctionProduct => 30020,
            Nip15Kind::Bid => 1021,
            Nip15Kind::BidConfirmation => 1022,
        }
    }
}

// ==================== NIP-15 Metadata Types ====================

/// Kind 30017: Create or update a stall
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StallMetadata {
    /// Unique stall ID (also used as d-tag)
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub currency: String,
    pub shipping: Vec<ShippingZone>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ShippingZone {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub cost: f64,
    pub regions: Vec<String>,
}

impl NipMetadata for StallMetadata {
    type Kind = Nip15Kind;

    fn kind(&self) -> Self::Kind {
        Nip15Kind::SetStall
    }

    fn validate(&self) -> NipResult<()> {
        if self.id.is_empty() {
            return Err(NipError::Nip15("Stall ID cannot be empty".into()));
        }
        if self.name.is_empty() {
            return Err(NipError::Nip15("Stall name cannot be empty".into()));
        }
        if self.currency.is_empty() {
            return Err(NipError::Nip15("Currency cannot be empty".into()));
        }
        if self.shipping.is_empty() {
            return Err(NipError::Nip15(
                "At least one shipping zone is required".into(),
            ));
        }
        for zone in &self.shipping {
            if zone.id.is_empty() {
                return Err(NipError::Nip15("Shipping zone ID cannot be empty".into()));
            }
            if zone.regions.is_empty() {
                return Err(NipError::Nip15(
                    "Shipping zone must have at least one region".into(),
                ));
            }
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        vec![Tag::d(&self.id)]
    }

    fn content(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn d_tag(&self) -> Option<String> {
        Some(self.id.clone())
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip15Kind::SetStall.kind_value() {
            return Err(NipError::Nip15(format!(
                "Expected kind 30017, got {}",
                event.kind
            )));
        }

        let d_tag = event
            .d_tag()
            .ok_or_else(|| NipError::Nip15("Stall event must have a d-tag".into()))?;

        let stall: StallMetadata =
            serde_json::from_str(&event.content).map_err(NipError::Serialization)?;

        // Ensure d-tag matches stall ID
        if stall.id != d_tag {
            return Err(NipError::Nip15(format!(
                "Stall ID {} does not match d-tag {}",
                stall.id, d_tag
            )));
        }

        Ok(stall)
    }
}

/// Kind 30018: Create or update a product
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProductMetadata {
    /// Unique product ID (also used as d-tag)
    pub id: String,
    pub stall_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    pub currency: String,
    pub price: f64,
    pub quantity: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub specs: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shipping: Vec<ProductShipping>,
    /// Product categories (t-tags)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProductShipping {
    pub id: String,
    pub cost: f64,
}

impl NipMetadata for ProductMetadata {
    type Kind = Nip15Kind;

    fn kind(&self) -> Self::Kind {
        Nip15Kind::SetProduct
    }

    fn validate(&self) -> NipResult<()> {
        if self.id.is_empty() {
            return Err(NipError::Nip15("Product ID cannot be empty".into()));
        }
        if self.stall_id.is_empty() {
            return Err(NipError::Nip15("Stall ID cannot be empty".into()));
        }
        if self.name.is_empty() {
            return Err(NipError::Nip15("Product name cannot be empty".into()));
        }
        if self.currency.is_empty() {
            return Err(NipError::Nip15("Currency cannot be empty".into()));
        }
        if self.price < 0.0 {
            return Err(NipError::Nip15("Price cannot be negative".into()));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        let mut tags = vec![Tag::d(&self.id)];

        // Add category t-tags
        for category in &self.categories {
            tags.push(Tag::t(category));
        }

        tags
    }

    fn content(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn d_tag(&self) -> Option<String> {
        Some(self.id.clone())
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip15Kind::SetProduct.kind_value() {
            return Err(NipError::Nip15(format!(
                "Expected kind 30018, got {}",
                event.kind
            )));
        }

        let d_tag = event
            .d_tag()
            .ok_or_else(|| NipError::Nip15("Product event must have a d-tag".into()))?;

        let mut product: ProductMetadata =
            serde_json::from_str(&event.content).map_err(NipError::Serialization)?;

        // Ensure d-tag matches product ID
        if product.id != d_tag {
            return Err(NipError::Nip15(format!(
                "Product ID {} does not match d-tag {}",
                product.id, d_tag
            )));
        }

        // Extract categories from t-tags
        let categories: Vec<String> = event
            .tags
            .iter()
            .filter(|tag| tag.first().map(|n| n.as_str()) == Some("t"))
            .filter_map(|tag| tag.get(1).cloned())
            .collect();

        product.categories = categories;

        Ok(product)
    }
}

/// Kind 30019: Marketplace UI/UX configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MarketplaceUIMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<MarketplaceUI>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merchants: Vec<String>,
    /// Unique identifier for this marketplace config
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MarketplaceUI {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dark_mode: Option<bool>,
}

impl NipMetadata for MarketplaceUIMetadata {
    type Kind = Nip15Kind;

    fn kind(&self) -> Self::Kind {
        Nip15Kind::MarketplaceUI
    }

    fn validate(&self) -> NipResult<()> {
        if self.id.is_empty() {
            return Err(NipError::Nip15("Marketplace ID cannot be empty".into()));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        let mut tags = vec![Tag::d(&self.id)];

        // Add merchant p-tags
        for merchant in &self.merchants {
            tags.push(Tag::p(merchant));
        }

        tags
    }

    fn content(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn d_tag(&self) -> Option<String> {
        Some(self.id.clone())
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip15Kind::MarketplaceUI.kind_value() {
            return Err(NipError::Nip15(format!(
                "Expected kind 30019, got {}",
                event.kind
            )));
        }

        let d_tag = event
            .d_tag()
            .ok_or_else(|| NipError::Nip15("Marketplace UI event must have a d-tag".into()))?;

        let mut ui: MarketplaceUIMetadata =
            serde_json::from_str(&event.content).map_err(NipError::Serialization)?;

        ui.id = d_tag.to_string();

        // Extract merchants from p-tags
        ui.merchants = event
            .tags
            .iter()
            .filter(|tag| tag.first().map(|n| n.as_str()) == Some("p"))
            .filter_map(|tag| tag.get(1).cloned())
            .collect();

        Ok(ui)
    }
}

/// Kind 30020: Auction product
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuctionProductMetadata {
    pub id: String,
    pub stall_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    pub starting_bid: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<u64>,
    pub duration: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub specs: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shipping: Vec<ProductShipping>,
}

impl NipMetadata for AuctionProductMetadata {
    type Kind = Nip15Kind;

    fn kind(&self) -> Self::Kind {
        Nip15Kind::AuctionProduct
    }

    fn validate(&self) -> NipResult<()> {
        if self.id.is_empty() {
            return Err(NipError::Nip15("Auction ID cannot be empty".into()));
        }
        if self.name.is_empty() {
            return Err(NipError::Nip15("Auction name cannot be empty".into()));
        }
        if self.duration == 0 {
            return Err(NipError::Nip15("Duration must be greater than 0".into()));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        vec![Tag::d(&self.id)]
    }

    fn content(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn d_tag(&self) -> Option<String> {
        Some(self.id.clone())
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip15Kind::AuctionProduct.kind_value() {
            return Err(NipError::Nip15(format!(
                "Expected kind 30020, got {}",
                event.kind
            )));
        }

        let d_tag = event
            .d_tag()
            .ok_or_else(|| NipError::Nip15("Auction event must have a d-tag".into()))?;

        let auction: AuctionProductMetadata =
            serde_json::from_str(&event.content).map_err(NipError::Serialization)?;

        if auction.id != d_tag {
            return Err(NipError::Nip15(format!(
                "Auction ID {} does not match d-tag {}",
                auction.id, d_tag
            )));
        }

        Ok(auction)
    }
}

/// Kind 1021: Bid
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BidMetadata {
    /// The bid amount in sats
    pub amount: u64,
    /// The event ID of the auction being bid on
    pub auction_event_id: String,
}

impl NipMetadata for BidMetadata {
    type Kind = Nip15Kind;

    fn kind(&self) -> Self::Kind {
        Nip15Kind::Bid
    }

    fn validate(&self) -> NipResult<()> {
        if self.auction_event_id.is_empty() {
            return Err(NipError::Nip15("Auction event ID cannot be empty".into()));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        vec![Tag::e(&self.auction_event_id)]
    }

    fn content(&self) -> String {
        self.amount.to_string()
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip15Kind::Bid.kind_value() {
            return Err(NipError::Nip15(format!(
                "Expected kind 1021, got {}",
                event.kind
            )));
        }

        let auction_event_id = event
            .tags
            .iter()
            .find(|tag| tag.first().map(|n| n.as_str()) == Some("e"))
            .and_then(|tag| tag.get(1).cloned())
            .ok_or_else(|| NipError::Nip15("Bid must reference an auction event".into()))?;

        let amount: u64 = event
            .content
            .parse()
            .map_err(|_| NipError::Nip15("Bid content must be a numeric amount".into()))?;

        Ok(BidMetadata {
            amount,
            auction_event_id,
        })
    }
}

/// Kind 1022: Bid confirmation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BidConfirmationMetadata {
    pub status: BidStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_extended: Option<u64>,
    /// The event ID of the bid being confirmed
    pub bid_event_id: String,
    /// The event ID of the auction
    pub auction_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BidStatus {
    Accepted,
    Rejected,
    Pending,
    Winner,
}

impl NipMetadata for BidConfirmationMetadata {
    type Kind = Nip15Kind;

    fn kind(&self) -> Self::Kind {
        Nip15Kind::BidConfirmation
    }

    fn validate(&self) -> NipResult<()> {
        if self.bid_event_id.is_empty() {
            return Err(NipError::Nip15("Bid event ID cannot be empty".into()));
        }
        if self.auction_event_id.is_empty() {
            return Err(NipError::Nip15("Auction event ID cannot be empty".into()));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        vec![Tag::e(&self.bid_event_id), Tag::e(&self.auction_event_id)]
    }

    fn content(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip15Kind::BidConfirmation.kind_value() {
            return Err(NipError::Nip15(format!(
                "Expected kind 1022, got {}",
                event.kind
            )));
        }

        let e_tags: Vec<&Vec<String>> = event
            .tags
            .iter()
            .filter(|tag| tag.first().map(|n| n.as_str()) == Some("e"))
            .collect();

        if e_tags.len() < 2 {
            return Err(NipError::Nip15(
                "Bid confirmation must reference both bid and auction events".into(),
            ));
        }

        let bid_event_id = e_tags[0]
            .get(1)
            .ok_or_else(|| NipError::Nip15("Missing bid event ID".into()))?
            .clone();
        let auction_event_id = e_tags[1]
            .get(1)
            .ok_or_else(|| NipError::Nip15("Missing auction event ID".into()))?
            .clone();

        let mut confirmation: BidConfirmationMetadata =
            serde_json::from_str(&event.content).map_err(NipError::Serialization)?;

        confirmation.bid_event_id = bid_event_id;
        confirmation.auction_event_id = auction_event_id;

        Ok(confirmation)
    }
}

// ==================== Checkout Message Types ====================

/// Customer order message (type 0)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CustomerOrder {
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<OrderContact>,
    pub items: Vec<OrderItem>,
    pub shipping_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrderContact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nostr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrderItem {
    pub product_id: String,
    pub quantity: u64,
}

impl CustomerOrder {
    pub fn new(
        id: impl Into<String>,
        items: Vec<OrderItem>,
        shipping_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            message_type: 0,
            name: None,
            address: None,
            message: None,
            contact: None,
            items,
            shipping_id: shipping_id.into(),
        }
    }
}

/// Merchant payment request message (type 1)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PaymentRequest {
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub payment_options: Vec<PaymentOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PaymentOption {
    #[serde(rename = "type")]
    pub payment_type: PaymentType,
    pub link: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaymentType {
    Url,
    Btc,
    Ln,
    Lnurl,
}

impl PaymentRequest {
    pub fn new(id: impl Into<String>, payment_options: Vec<PaymentOption>) -> Self {
        Self {
            id: id.into(),
            message_type: 1,
            message: None,
            payment_options,
        }
    }
}

/// Merchant order status update message (type 2)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrderStatusUpdate {
    pub id: String,
    #[serde(rename = "type")]
    pub message_type: u8,
    pub message: String,
    pub paid: bool,
    pub shipped: bool,
}

impl OrderStatusUpdate {
    pub fn new(
        id: impl Into<String>,
        message: impl Into<String>,
        paid: bool,
        shipped: bool,
    ) -> Self {
        Self {
            id: id.into(),
            message_type: 2,
            message: message.into(),
            paid,
            shipped,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NipKind, NipMetadata, RawNostrEvent};

    // ==================== Nip15Kind Tests ====================

    #[test]
    fn test_nip15kind_kind_values() {
        assert_eq!(Nip15Kind::SetStall.kind_value(), 30017);
        assert_eq!(Nip15Kind::SetProduct.kind_value(), 30018);
        assert_eq!(Nip15Kind::MarketplaceUI.kind_value(), 30019);
        assert_eq!(Nip15Kind::AuctionProduct.kind_value(), 30020);
        assert_eq!(Nip15Kind::Bid.kind_value(), 1021);
        assert_eq!(Nip15Kind::BidConfirmation.kind_value(), 1022);
    }

    #[test]
    fn test_nip15kind_kind_method() {
        assert_eq!(make_valid_stall().kind(), Nip15Kind::SetStall);
        assert_eq!(make_valid_product().kind(), Nip15Kind::SetProduct);
        assert_eq!(make_valid_marketplace_ui().kind(), Nip15Kind::MarketplaceUI);
        assert_eq!(make_valid_auction().kind(), Nip15Kind::AuctionProduct);
        assert_eq!(make_valid_bid().kind(), Nip15Kind::Bid);
        assert_eq!(
            make_valid_bid_confirmation(BidStatus::Accepted).kind(),
            Nip15Kind::BidConfirmation
        );
    }

    // ==================== StallMetadata Tests ====================

    fn make_valid_stall() -> StallMetadata {
        StallMetadata {
            id: "stall-1".into(),
            name: "My Stall".into(),
            description: Some("A nice stall".into()),
            currency: "USD".into(),
            shipping: vec![ShippingZone {
                id: "zone-1".into(),
                name: Some("US".into()),
                cost: 5.0,
                regions: vec!["US".into()],
            }],
        }
    }

    #[test]
    fn test_stall_validate_passes() {
        let stall = make_valid_stall();
        assert!(stall.validate().is_ok());
    }

    #[test]
    fn test_stall_validate_empty_id_fails() {
        let mut stall = make_valid_stall();
        stall.id = String::new();
        let err = stall.validate().unwrap_err();
        assert!(err.to_string().contains("Stall ID cannot be empty"));
    }

    #[test]
    fn test_stall_validate_empty_name_fails() {
        let mut stall = make_valid_stall();
        stall.name = String::new();
        let err = stall.validate().unwrap_err();
        assert!(err.to_string().contains("Stall name cannot be empty"));
    }

    #[test]
    fn test_stall_validate_empty_currency_fails() {
        let mut stall = make_valid_stall();
        stall.currency = String::new();
        let err = stall.validate().unwrap_err();
        assert!(err.to_string().contains("Currency cannot be empty"));
    }

    #[test]
    fn test_stall_validate_no_shipping_zones_fails() {
        let mut stall = make_valid_stall();
        stall.shipping = vec![];
        let err = stall.validate().unwrap_err();
        assert!(
            err.to_string()
                .contains("At least one shipping zone is required")
        );
    }

    #[test]
    fn test_stall_validate_empty_zone_id_fails() {
        let mut stall = make_valid_stall();
        stall.shipping[0].id = String::new();
        let err = stall.validate().unwrap_err();
        assert!(err.to_string().contains("Shipping zone ID cannot be empty"));
    }

    #[test]
    fn test_stall_validate_empty_zone_regions_fails() {
        let mut stall = make_valid_stall();
        stall.shipping[0].regions = vec![];
        let err = stall.validate().unwrap_err();
        assert!(
            err.to_string()
                .contains("Shipping zone must have at least one region")
        );
    }

    #[test]
    fn test_stall_to_tags_includes_d_tag() {
        let stall = make_valid_stall();
        let tags = stall.to_tags();
        assert!(tags.iter().any(|t| {
            let inner = t.as_slice();
            inner.first() == Some(&"d".to_string()) && inner.get(1) == Some(&"stall-1".to_string())
        }));
    }

    #[test]
    fn test_stall_from_raw_event_with_d_tag_match() {
        let stall = make_valid_stall();
        let content = serde_json::to_string(&stall).unwrap();
        let kind = Nip15Kind::SetStall.kind_value();
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind,
            tags: vec![vec!["d".into(), "stall-1".into()]],
            content,
            sig: "sig".into(),
        };
        let result = StallMetadata::from_raw_event(&event).unwrap();
        assert_eq!(result.id, "stall-1");
    }

    #[test]
    fn test_stall_from_raw_event_wrong_kind_fails() {
        let content = serde_json::to_string(&make_valid_stall()).unwrap();
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 30018,
            tags: vec![vec!["d".into(), "stall-1".into()]],
            content,
            sig: "sig".into(),
        };
        let err = StallMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("Expected kind 30017"));
    }

    #[test]
    fn test_stall_from_raw_event_missing_d_tag_fails() {
        let content = serde_json::to_string(&make_valid_stall()).unwrap();
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 30017,
            tags: vec![],
            content,
            sig: "sig".into(),
        };
        let err = StallMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("Stall event must have a d-tag"));
    }

    #[test]
    fn test_stall_from_raw_event_d_tag_mismatch_fails() {
        let content = serde_json::to_string(&make_valid_stall()).unwrap();
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 30017,
            tags: vec![vec!["d".into(), "different-id".into()]],
            content,
            sig: "sig".into(),
        };
        let err = StallMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("does not match d-tag"));
    }

    // ==================== ProductMetadata Tests ====================

    fn make_valid_product() -> ProductMetadata {
        ProductMetadata {
            id: "product-1".into(),
            stall_id: "stall-1".into(),
            name: "Widget".into(),
            description: Some("A widget".into()),
            images: vec![],
            currency: "USD".into(),
            price: 10.0,
            quantity: Some(100),
            specs: vec![],
            shipping: vec![],
            categories: vec!["electronics".into(), "gadgets".into()],
        }
    }

    #[test]
    fn test_product_validate_passes() {
        let product = make_valid_product();
        assert!(product.validate().is_ok());
    }

    #[test]
    fn test_product_validate_empty_id_fails() {
        let mut p = make_valid_product();
        p.id = String::new();
        let err = p.validate().unwrap_err();
        assert!(err.to_string().contains("Product ID cannot be empty"));
    }

    #[test]
    fn test_product_validate_negative_price_fails() {
        let mut p = make_valid_product();
        p.price = -1.0;
        let err = p.validate().unwrap_err();
        assert!(err.to_string().contains("Price cannot be negative"));
    }

    #[test]
    fn test_product_to_tags_includes_d_tag_and_t_tags() {
        let product = make_valid_product();
        let tags = product.to_tags();
        // Check d-tag
        assert!(tags.iter().any(|t| {
            let inner = t.as_slice();
            inner.first() == Some(&"d".to_string())
                && inner.get(1) == Some(&"product-1".to_string())
        }));
        // Check t-tags for categories
        assert!(tags.iter().any(|t| {
            let inner = t.as_slice();
            inner.first() == Some(&"t".to_string())
                && inner.get(1) == Some(&"electronics".to_string())
        }));
        assert!(tags.iter().any(|t| {
            let inner = t.as_slice();
            inner.first() == Some(&"t".to_string()) && inner.get(1) == Some(&"gadgets".to_string())
        }));
    }

    #[test]
    fn test_product_from_raw_event_extracts_categories() {
        let product = make_valid_product();
        let content = serde_json::to_string(&product).unwrap();
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 30018,
            tags: vec![
                vec!["d".into(), "product-1".into()],
                vec!["t".into(), "electronics".into()],
                vec!["t".into(), "gadgets".into()],
            ],
            content,
            sig: "sig".into(),
        };
        let result = ProductMetadata::from_raw_event(&event).unwrap();
        assert_eq!(result.id, "product-1");
        assert_eq!(result.categories, vec!["electronics", "gadgets"]);
    }

    // ==================== MarketplaceUIMetadata Tests ====================

    fn make_valid_marketplace_ui() -> MarketplaceUIMetadata {
        MarketplaceUIMetadata {
            name: Some("My Market".into()),
            about: Some("Best marketplace".into()),
            ui: Some(MarketplaceUI {
                picture: Some("pic.png".into()),
                banner: Some("banner.png".into()),
                theme: Some("dark".into()),
                dark_mode: Some(true),
            }),
            merchants: vec!["pk1".into(), "pk2".into()],
            id: "market-1".into(),
        }
    }

    #[test]
    fn test_marketplace_ui_to_tags_includes_d_tag_and_p_tags() {
        let ui = make_valid_marketplace_ui();
        let tags = ui.to_tags();
        // Check d-tag
        assert!(tags.iter().any(|t| {
            let inner = t.as_slice();
            inner.first() == Some(&"d".to_string()) && inner.get(1) == Some(&"market-1".to_string())
        }));
        // Check p-tags for merchants
        assert!(tags.iter().any(|t| {
            let inner = t.as_slice();
            inner.first() == Some(&"p".to_string()) && inner.get(1) == Some(&"pk1".to_string())
        }));
        assert!(tags.iter().any(|t| {
            let inner = t.as_slice();
            inner.first() == Some(&"p".to_string()) && inner.get(1) == Some(&"pk2".to_string())
        }));
    }

    #[test]
    fn test_marketplace_ui_from_raw_event_extracts_merchants() {
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 30019,
            tags: vec![
                vec!["d".into(), "market-1".into()],
                vec!["p".into(), "pk1".into()],
                vec!["p".into(), "pk2".into()],
            ],
            content: r#"{"name":"My Market","about":"Best marketplace","id":"market-1"}"#.into(),
            sig: "sig".into(),
        };
        let result = MarketplaceUIMetadata::from_raw_event(&event).unwrap();
        assert_eq!(result.id, "market-1");
        assert_eq!(result.merchants, vec!["pk1", "pk2"]);
        assert_eq!(result.name, Some("My Market".to_string()));
    }

    // ==================== AuctionProductMetadata Tests ====================

    fn make_valid_auction() -> AuctionProductMetadata {
        AuctionProductMetadata {
            id: "auction-1".into(),
            stall_id: "stall-1".into(),
            name: "Rare Widget".into(),
            description: Some("Very rare".into()),
            images: vec![],
            starting_bid: 1000,
            start_date: Some(1000000),
            duration: 86400,
            specs: vec![],
            shipping: vec![],
        }
    }

    #[test]
    fn test_auction_validate_passes() {
        let a = make_valid_auction();
        assert!(a.validate().is_ok());
    }

    #[test]
    fn test_auction_validate_empty_id_fails() {
        let mut a = make_valid_auction();
        a.id = String::new();
        let err = a.validate().unwrap_err();
        assert!(err.to_string().contains("Auction ID cannot be empty"));
    }

    #[test]
    fn test_auction_validate_empty_name_fails() {
        let mut a = make_valid_auction();
        a.name = String::new();
        let err = a.validate().unwrap_err();
        assert!(err.to_string().contains("Auction name cannot be empty"));
    }

    #[test]
    fn test_auction_validate_zero_duration_fails() {
        let mut a = make_valid_auction();
        a.duration = 0;
        let err = a.validate().unwrap_err();
        assert!(err.to_string().contains("Duration must be greater than 0"));
    }

    #[test]
    fn test_auction_from_raw_event_with_matching_d_tag() {
        let auction = make_valid_auction();
        let content = serde_json::to_string(&auction).unwrap();
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 30020,
            tags: vec![vec!["d".into(), "auction-1".into()]],
            content,
            sig: "sig".into(),
        };
        let result = AuctionProductMetadata::from_raw_event(&event).unwrap();
        assert_eq!(result.id, "auction-1");
        assert_eq!(result.name, "Rare Widget");
    }

    #[test]
    fn test_auction_from_raw_event_wrong_kind_fails() {
        let content = serde_json::to_string(&make_valid_auction()).unwrap();
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 30017,
            tags: vec![vec!["d".into(), "auction-1".into()]],
            content,
            sig: "sig".into(),
        };
        let err = AuctionProductMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("Expected kind 30020"));
    }

    #[test]
    fn test_auction_from_raw_event_missing_d_tag_fails() {
        let content = serde_json::to_string(&make_valid_auction()).unwrap();
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 30020,
            tags: vec![],
            content,
            sig: "sig".into(),
        };
        let err = AuctionProductMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("Auction event must have a d-tag"));
    }

    // ==================== BidMetadata Tests ====================

    fn make_valid_bid() -> BidMetadata {
        BidMetadata {
            amount: 5000,
            auction_event_id: "auction-event-1".into(),
        }
    }

    #[test]
    fn test_bid_validate_passes() {
        let bid = make_valid_bid();
        assert!(bid.validate().is_ok());
    }

    #[test]
    fn test_bid_validate_empty_auction_event_id_fails() {
        let mut bid = make_valid_bid();
        bid.auction_event_id = String::new();
        let err = bid.validate().unwrap_err();
        assert!(err.to_string().contains("Auction event ID cannot be empty"));
    }

    #[test]
    fn test_bid_from_raw_event_extracts_amount_and_event() {
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 1021,
            tags: vec![vec!["e".into(), "auction-event-1".into()]],
            content: "5000".into(),
            sig: "sig".into(),
        };
        let result = BidMetadata::from_raw_event(&event).unwrap();
        assert_eq!(result.amount, 5000);
        assert_eq!(result.auction_event_id, "auction-event-1");
    }

    #[test]
    fn test_bid_from_raw_event_wrong_kind_fails() {
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 1022,
            tags: vec![vec!["e".into(), "auction-event-1".into()]],
            content: "5000".into(),
            sig: "sig".into(),
        };
        let err = BidMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("Expected kind 1021"));
    }

    #[test]
    fn test_bid_from_raw_event_missing_e_tag_fails() {
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 1021,
            tags: vec![],
            content: "5000".into(),
            sig: "sig".into(),
        };
        let err = BidMetadata::from_raw_event(&event).unwrap_err();
        assert!(
            err.to_string()
                .contains("Bid must reference an auction event")
        );
    }

    #[test]
    fn test_bid_from_raw_event_non_numeric_content_fails() {
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 1021,
            tags: vec![vec!["e".into(), "auction-event-1".into()]],
            content: "not-a-number".into(),
            sig: "sig".into(),
        };
        let err = BidMetadata::from_raw_event(&event).unwrap_err();
        assert!(
            err.to_string()
                .contains("Bid content must be a numeric amount")
        );
    }

    // ==================== BidConfirmationMetadata Tests ====================

    fn make_valid_bid_confirmation(status: BidStatus) -> BidConfirmationMetadata {
        BidConfirmationMetadata {
            status,
            message: Some("Confirmed".into()),
            duration_extended: None,
            bid_event_id: "bid-event-1".into(),
            auction_event_id: "auction-event-1".into(),
        }
    }

    #[test]
    fn test_bid_confirmation_validate_accepted_passes() {
        let bc = make_valid_bid_confirmation(BidStatus::Accepted);
        assert!(bc.validate().is_ok());
    }

    #[test]
    fn test_bid_confirmation_validate_rejected_passes() {
        let bc = make_valid_bid_confirmation(BidStatus::Rejected);
        assert!(bc.validate().is_ok());
    }

    #[test]
    fn test_bid_confirmation_validate_empty_bid_event_id_fails() {
        let mut bc = make_valid_bid_confirmation(BidStatus::Accepted);
        bc.bid_event_id = String::new();
        let err = bc.validate().unwrap_err();
        assert!(err.to_string().contains("Bid event ID cannot be empty"));
    }

    #[test]
    fn test_bid_confirmation_validate_empty_auction_event_id_fails() {
        let mut bc = make_valid_bid_confirmation(BidStatus::Accepted);
        bc.auction_event_id = String::new();
        let err = bc.validate().unwrap_err();
        assert!(err.to_string().contains("Auction event ID cannot be empty"));
    }

    #[test]
    fn test_bid_confirmation_to_tags_produces_two_e_tags() {
        let bc = make_valid_bid_confirmation(BidStatus::Accepted);
        let tags = bc.to_tags();
        assert_eq!(tags.len(), 2);
        assert!(tags.iter().all(|t| {
            let inner = t.as_slice();
            inner.first() == Some(&"e".to_string())
        }));
        assert!(tags.iter().any(|t| {
            let inner = t.as_slice();
            inner.get(1) == Some(&"bid-event-1".to_string())
        }));
        assert!(tags.iter().any(|t| {
            let inner = t.as_slice();
            inner.get(1) == Some(&"auction-event-1".to_string())
        }));
    }

    #[test]
    fn test_bid_confirmation_from_raw_event() {
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 1022,
            tags: vec![
                vec!["e".into(), "bid-event-1".into()],
                vec!["e".into(), "auction-event-1".into()],
            ],
            content: r#"{"status":"accepted","message":"Confirmed","bid_event_id":"x","auction_event_id":"y"}"#.into(),
            sig: "sig".into(),
        };
        let result = BidConfirmationMetadata::from_raw_event(&event).unwrap();
        assert_eq!(result.status, BidStatus::Accepted);
        assert_eq!(result.bid_event_id, "bid-event-1");
        assert_eq!(result.auction_event_id, "auction-event-1");
        assert_eq!(result.message, Some("Confirmed".to_string()));
    }

    #[test]
    fn test_bid_confirmation_from_raw_event_wrong_kind_fails() {
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 1021,
            tags: vec![
                vec!["e".into(), "bid-event-1".into()],
                vec!["e".into(), "auction-event-1".into()],
            ],
            content: "{}".into(),
            sig: "sig".into(),
        };
        let err = BidConfirmationMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("Expected kind 1022"));
    }

    #[test]
    fn test_bid_confirmation_from_raw_event_less_than_two_e_tags_fails() {
        let event = RawNostrEvent {
            id: "event-id".into(),
            pubkey: "pk".into(),
            created_at: 1000,
            kind: 1022,
            tags: vec![vec!["e".into(), "bid-event-1".into()]],
            content: "{}".into(),
            sig: "sig".into(),
        };
        let err = BidConfirmationMetadata::from_raw_event(&event).unwrap_err();
        assert!(
            err.to_string()
                .contains("Bid confirmation must reference both bid and auction events")
        );
    }

    // ==================== Checkout Message Constructor Tests ====================

    #[test]
    fn test_customer_order_new() {
        let items = vec![OrderItem {
            product_id: "product-1".into(),
            quantity: 2,
        }];
        let order = CustomerOrder::new("order-1", items.clone(), "shipping-1");
        assert_eq!(order.id, "order-1");
        assert_eq!(order.message_type, 0);
        assert_eq!(order.items.len(), 1);
        assert_eq!(order.items[0].product_id, "product-1");
        assert_eq!(order.shipping_id, "shipping-1");
        assert!(order.name.is_none());
        assert!(order.address.is_none());
        assert!(order.message.is_none());
        assert!(order.contact.is_none());
    }

    #[test]
    fn test_payment_request_new() {
        let options = vec![PaymentOption {
            payment_type: PaymentType::Ln,
            link: "lnbc...".into(),
        }];
        let req = PaymentRequest::new("pay-1", options.clone());
        assert_eq!(req.id, "pay-1");
        assert_eq!(req.message_type, 1);
        assert_eq!(req.payment_options.len(), 1);
        assert_eq!(req.payment_options[0].payment_type, PaymentType::Ln);
    }

    #[test]
    fn test_order_status_update_new() {
        let update = OrderStatusUpdate::new("order-1", "Shipped", true, true);
        assert_eq!(update.id, "order-1");
        assert_eq!(update.message_type, 2);
        assert_eq!(update.message, "Shipped");
        assert!(update.paid);
        assert!(update.shipped);

        let unpaid = OrderStatusUpdate::new("order-2", "Pending", false, false);
        assert!(!unpaid.paid);
        assert!(!unpaid.shipped);
    }

    // ==================== Coverage Gap: content(), d_tag(), remaining validate/from_raw_event ====================

    #[test]
    fn test_stall_content_and_d_tag() {
        let s = make_valid_stall();
        let c = s.content();
        assert!(c.contains("stall-1"));
        assert!(c.contains("My Stall"));
        assert!(c.contains("USD"));
        assert_eq!(s.d_tag(), Some("stall-1".to_string()));
    }

    #[test]
    fn test_product_content_and_d_tag() {
        let p = make_valid_product();
        let c = p.content();
        assert!(c.contains("product-1"));
        assert!(c.contains("Widget"));
        assert!(c.contains("USD"));
        assert_eq!(p.d_tag(), Some("product-1".to_string()));
    }

    #[test]
    fn test_product_validate_empty_stall_id_fails() {
        let mut p = make_valid_product();
        p.stall_id = String::new();
        let err = p.validate().unwrap_err();
        assert!(err.to_string().contains("Stall ID cannot be empty"));
    }

    #[test]
    fn test_product_validate_empty_name_fails() {
        let mut p = make_valid_product();
        p.name = String::new();
        let err = p.validate().unwrap_err();
        assert!(err.to_string().contains("Product name cannot be empty"));
    }

    #[test]
    fn test_product_validate_empty_currency_fails() {
        let mut p = make_valid_product();
        p.currency = String::new();
        let err = p.validate().unwrap_err();
        assert!(err.to_string().contains("Currency cannot be empty"));
    }

    #[test]
    fn test_product_from_raw_event_wrong_kind_fails() {
        let content = serde_json::to_string(&make_valid_product()).unwrap();
        let event = RawNostrEvent {
            id: "eid".into(),
            pubkey: "pk".into(),
            created_at: 0,
            kind: 30017,
            tags: vec![vec!["d".into(), "product-1".into()]],
            content,
            sig: "sig".into(),
        };
        let err = ProductMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("Expected kind 30018"));
    }

    #[test]
    fn test_product_from_raw_event_d_tag_mismatch_fails() {
        let content = serde_json::to_string(&make_valid_product()).unwrap();
        let event = RawNostrEvent {
            id: "eid".into(),
            pubkey: "pk".into(),
            created_at: 0,
            kind: 30018,
            tags: vec![vec!["d".into(), "wrong-id".into()]],
            content,
            sig: "sig".into(),
        };
        let err = ProductMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("does not match d-tag"));
    }

    #[test]
    fn test_product_from_raw_event_missing_d_tag_fails() {
        let content = serde_json::to_string(&make_valid_product()).unwrap();
        let event = RawNostrEvent {
            id: "eid".into(),
            pubkey: "pk".into(),
            created_at: 0,
            kind: 30018,
            tags: vec![],
            content,
            sig: "sig".into(),
        };
        let err = ProductMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("Product event must have a d-tag"));
    }

    #[test]
    fn test_marketplace_ui_content_and_d_tag() {
        let ui = make_valid_marketplace_ui();
        let c = ui.content();
        assert!(c.contains("market-1"));
        assert!(c.contains("My Market"));
        assert_eq!(ui.d_tag(), Some("market-1".to_string()));
    }

    #[test]
    fn test_marketplace_ui_validate_passes() {
        let ui = make_valid_marketplace_ui();
        assert!(ui.validate().is_ok());
    }

    #[test]
    fn test_marketplace_ui_validate_empty_id_fails() {
        let mut ui = make_valid_marketplace_ui();
        ui.id = String::new();
        let err = ui.validate().unwrap_err();
        assert!(err.to_string().contains("Marketplace ID cannot be empty"));
    }

    #[test]
    fn test_marketplace_ui_from_raw_event_wrong_kind_fails() {
        let event = RawNostrEvent {
            id: "eid".into(),
            pubkey: "pk".into(),
            created_at: 0,
            kind: 30017,
            tags: vec![vec!["d".into(), "m1".into()]],
            content: r#"{"id":"m1"}"#.into(),
            sig: "sig".into(),
        };
        let err = MarketplaceUIMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("Expected kind 30019"));
    }

    #[test]
    fn test_auction_content_and_d_tag() {
        let a = make_valid_auction();
        let c = a.content();
        assert!(c.contains("auction-1"));
        assert!(c.contains("Rare Widget"));
        assert_eq!(a.d_tag(), Some("auction-1".to_string()));
    }

    #[test]
    fn test_auction_to_tags() {
        let a = make_valid_auction();
        let tags = a.to_tags();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name(), Some("d"));
        assert_eq!(tags[0].value(), Some("auction-1"));
    }

    #[test]
    fn test_auction_from_raw_event_d_tag_mismatch_fails() {
        let content = serde_json::to_string(&make_valid_auction()).unwrap();
        let event = RawNostrEvent {
            id: "eid".into(),
            pubkey: "pk".into(),
            created_at: 0,
            kind: 30020,
            tags: vec![vec!["d".into(), "wrong-id".into()]],
            content,
            sig: "sig".into(),
        };
        let err = AuctionProductMetadata::from_raw_event(&event).unwrap_err();
        assert!(err.to_string().contains("does not match d-tag"));
    }

    #[test]
    fn test_bid_content_and_to_tags() {
        let bid = make_valid_bid();
        assert_eq!(bid.content(), "5000");
        let tags = bid.to_tags();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name(), Some("e"));
        assert_eq!(tags[0].value(), Some("auction-event-1"));
    }

    #[test]
    fn test_bid_confirmation_content() {
        let bc = make_valid_bid_confirmation(BidStatus::Accepted);
        let c = bc.content();
        assert!(c.contains("accepted"));
        assert!(c.contains("Confirmed"));
    }

    #[test]
    fn test_bid_confirmation_kind() {
        assert_eq!(
            make_valid_bid_confirmation(BidStatus::Accepted).kind(),
            Nip15Kind::BidConfirmation
        );
    }
}
