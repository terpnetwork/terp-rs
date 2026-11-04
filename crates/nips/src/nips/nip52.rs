use crate::{
    NipKind, NipMetadata, RawNostrEvent, Tag,
    error::{NipError, NipResult},
    types::*,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

// ==================== NIP-52 Kinds ====================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Nip52Kind {
    DateEvent,
    TimeEvent,
    Calendar,
    RSVP,
}

impl NipKind for Nip52Kind {
    fn kind_value(&self) -> u16 {
        match self {
            Nip52Kind::DateEvent => 31922,
            Nip52Kind::TimeEvent => 31923,
            Nip52Kind::Calendar => 31924,
            Nip52Kind::RSVP => 31925,
        }
    }
}

// ==================== NIP-52 Common Types ====================

/// Reference to another calendar or event
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct CalendarReference {
    pub kind: u32,
    pub author_pubkey: String,
    pub d_identifier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
}

impl CalendarReference {
    pub fn new(
        kind: u32,
        author_pubkey: impl Into<String>,
        d_identifier: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            author_pubkey: author_pubkey.into(),
            d_identifier: d_identifier.into(),
            relay_url: None,
        }
    }

    pub fn with_relay(mut self, relay: impl Into<String>) -> Self {
        self.relay_url = Some(relay.into());
        self
    }

    /// Convert to an "a" tag
    pub fn to_a_tag(&self) -> Tag {
        let _a_value = format!("{}:{}:{}", self.kind, self.author_pubkey, self.d_identifier);
        if let Some(ref relay) = self.relay_url {
            Tag::a_with_relay(
                self.kind as u16,
                &self.author_pubkey,
                &self.d_identifier,
                relay,
            )
        } else {
            Tag::a(self.kind as u16, &self.author_pubkey, &self.d_identifier)
        }
    }

    /// Parse from an "a" tag
    pub fn from_a_tag(tag: &Tag) -> NipResult<Self> {
        let value = tag
            .value()
            .ok_or_else(|| NipError::Tag("Missing tag value".into()))?;
        let parts: Vec<&str> = value.split(':').collect();
        if parts.len() < 3 {
            return Err(NipError::Nip52("Invalid a-tag format".into()));
        }
        let kind: u32 = parts[0]
            .parse()
            .map_err(|_| NipError::Nip52("Invalid kind in a-tag".into()))?;
        let relay_url = tag.additional().first().cloned();
        Ok(Self {
            kind,
            author_pubkey: parts[1].to_string(),
            d_identifier: parts[2].to_string(),
            relay_url,
        })
    }
}

/// RSVP status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RSVPStatus {
    Accepted,
    Declined,
    Tentative,
}

impl RSVPStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RSVPStatus::Accepted => "accepted",
            RSVPStatus::Declined => "declined",
            RSVPStatus::Tentative => "tentative",
        }
    }
}

/// Free/busy status
#[derive(Debug, Clone, Copy, PartialEq, JsonSchema, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FreeBusyStatus {
    Free,
    Busy,
}

impl FreeBusyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            FreeBusyStatus::Free => "free",
            FreeBusyStatus::Busy => "busy",
        }
    }
}

/// Event type discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema, Deserialize)]
pub enum EventType {
    #[serde(rename = "time")]
    TimeBased,
    #[serde(rename = "date")]
    DateBased,
}

impl EventType {
    pub fn kind(&self) -> Nip52Kind {
        match self {
            EventType::TimeBased => Nip52Kind::TimeEvent,
            EventType::DateBased => Nip52Kind::DateEvent,
        }
    }
}

/// NIP-52 Calendar Event metadata
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct CalendarEventMetadata {
    /// d-tag: short unique identifier
    pub d_tag: String,
    /// Event title
    pub title: String,
    /// Brief description (summary)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Image URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Full description (content field)
    pub content: String,
    /// Location strings
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<String>,
    /// Geohash for physical location (g-tag)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geohash: Option<String>,
    /// References/links (r-tags)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    /// Hashtags for categorization (t-tags)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashtags: Vec<String>,
    /// Start timezone (IANA identifier)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_timezone: Option<String>,
    /// End timezone (IANA identifier)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_timezone: Option<String>,
    /// Day-granularity timestamps (kind:31923 only)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub day_granularity: Vec<u64>,
    /// Collaborative calendar references (a-tags)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calendar_requests: Vec<CalendarReference>,
    /// Whether this is a time-based or date-based event
    pub event_type: EventType,
    /// Start timestamp (unix seconds)
    pub start_time: u64,
    /// End timestamp (unix seconds)
    pub end_time: u64,
    /// Participants (p-tags)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<Participant>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Participant {
    pub pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

impl CalendarEventMetadata {
    pub fn new_time_based(
        d_tag: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
        start_time: u64,
        end_time: u64,
    ) -> Self {
        Self {
            d_tag: d_tag.into(),
            title: title.into(),
            summary: None,
            image: None,
            content: content.into(),
            locations: Vec::new(),
            geohash: None,
            references: Vec::new(),
            hashtags: Vec::new(),
            start_timezone: None,
            end_timezone: None,
            day_granularity: Vec::new(),
            calendar_requests: Vec::new(),
            event_type: EventType::TimeBased,
            start_time,
            end_time,
            participants: Vec::new(),
        }
    }

    pub fn new_date_based(
        d_tag: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
        start_time: u64,
        end_time: u64,
    ) -> Self {
        Self {
            d_tag: d_tag.into(),
            title: title.into(),
            summary: None,
            image: None,
            content: content.into(),
            locations: Vec::new(),
            geohash: None,
            references: Vec::new(),
            hashtags: Vec::new(),
            start_timezone: None,
            end_timezone: None,
            day_granularity: Vec::new(),
            calendar_requests: Vec::new(),
            event_type: EventType::DateBased,
            start_time,
            end_time,
            participants: Vec::new(),
        }
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn with_image(mut self, image: impl Into<String>) -> Self {
        self.image = Some(image.into());
        self
    }

    pub fn with_location(mut self, location: impl Into<String>) -> Self {
        self.locations.push(location.into());
        self
    }

    pub fn with_geohash(mut self, geohash: impl Into<String>) -> Self {
        self.geohash = Some(geohash.into());
        self
    }

    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.references.push(reference.into());
        self
    }

    pub fn with_hashtag(mut self, hashtag: impl Into<String>) -> Self {
        self.hashtags.push(hashtag.into());
        self
    }

    pub fn with_calendar_request(mut self, reference: CalendarReference) -> Self {
        self.calendar_requests.push(reference);
        self
    }

    pub fn with_participant(mut self, pubkey: impl Into<String>, role: Option<String>) -> Self {
        self.participants.push(Participant {
            pubkey: pubkey.into(),
            relay_url: None,
            role,
        });
        self
    }
}

impl NipMetadata for CalendarEventMetadata {
    type Kind = Nip52Kind;

    fn kind(&self) -> Self::Kind {
        self.event_type.kind()
    }

    fn validate(&self) -> NipResult<()> {
        if self.d_tag.is_empty() {
            return Err(NipError::Nip52("d-tag cannot be empty".into()));
        }
        if self.title.is_empty() {
            return Err(NipError::Nip52("Title cannot be empty".into()));
        }
        if self.start_time > self.end_time {
            return Err(NipError::Nip52("Start time must be before end time".into()));
        }
        if self.event_type == EventType::TimeBased && self.day_granularity.is_empty() {
            // Time-based events should have day granularity timestamps
            // This is a warning, not an error
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        let mut tags = vec![Tag::d(&self.d_tag)];

        // Title tag
        tags.push(Tag::new(vec![String::from("title"), self.title.clone()]));

        // Summary tag (if present)
        if let Some(ref summary) = self.summary {
            tags.push(Tag::new(vec![String::from("summary"), summary.clone()]));
        }

        // Image tag (if present)
        if let Some(ref image) = self.image {
            tags.push(Tag::new(vec![String::from("image"), image.clone()]));
        }

        // Start/end time tags
        tags.push(Tag::new(vec![
            String::from("start"),
            self.start_time.to_string(),
        ]));
        if let Some(ref tz) = self.start_timezone {
            tags.push(Tag::new(vec![String::from("start_tzid"), tz.clone()]));
        }

        tags.push(Tag::new(vec![
            String::from("end"),
            self.end_time.to_string(),
        ]));
        if let Some(ref tz) = self.end_timezone {
            tags.push(Tag::new(vec![String::from("end_tzid"), tz.clone()]));
        }

        // Location tags
        for location in &self.locations {
            tags.push(Tag::new(vec![String::from("location"), location.clone()]));
        }

        // Geohash tag
        if let Some(ref geohash) = self.geohash {
            tags.push(Tag::g(geohash));
        }

        // Reference tags
        for reference in &self.references {
            tags.push(Tag::r(reference));
        }

        // Hashtag tags
        for hashtag in &self.hashtags {
            tags.push(Tag::t(hashtag));
        }

        // Day granularity tags (for time-based events)
        for day in &self.day_granularity {
            tags.push(Tag::new(vec![String::from("D"), day.to_string()]));
        }

        // Calendar request tags (a-tags)
        for request in &self.calendar_requests {
            tags.push(request.to_a_tag());
        }

        // Participant tags (p-tags)
        for participant in &self.participants {
            let mut p_tag = Tag::p(&participant.pubkey);
            if let Some(ref relay) = participant.relay_url {
                p_tag = Tag::p_with_relay(&participant.pubkey, relay);
            }
            tags.push(p_tag);
            if let Some(ref role) = participant.role {
                tags.push(Tag::new(vec![
                    String::from("role"),
                    participant.pubkey.clone(),
                    role.clone(),
                ]));
            }
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
        let event_type = match event.kind {
            31922 => EventType::DateBased,
            31923 => EventType::TimeBased,
            _ => {
                return Err(NipError::Nip52(format!(
                    "Expected kind 31922 or 31923, got {}",
                    event.kind
                )));
            }
        };

        let d_tag = event
            .d_tag()
            .ok_or_else(|| NipError::Nip52("Calendar event must have a d-tag".into()))?
            .to_string();

        let mut title = String::new();
        let mut summary = None;
        let mut image = None;
        let mut start_time = 0u64;
        let mut end_time = 0u64;
        let mut start_timezone = None;
        let mut end_timezone = None;
        let mut locations = Vec::new();
        let mut geohash = None;
        let mut references = Vec::new();
        let mut hashtags = Vec::new();
        let day_granularity = Vec::new();
        let mut calendar_requests = Vec::new();
        let mut participants = Vec::new();

        for tag in &event.tags {
            if tag.is_empty() {
                continue;
            }
            match tag[0].as_str() {
                "title" => {
                    if let Some(v) = tag.get(1) {
                        title = v.clone();
                    }
                }
                "summary" => {
                    if let Some(v) = tag.get(1) {
                        summary = Some(v.clone());
                    }
                }
                "image" => {
                    if let Some(v) = tag.get(1) {
                        image = Some(v.clone());
                    }
                }
                "start" => {
                    if let Some(v) = tag.get(1) {
                        start_time = v
                            .parse()
                            .map_err(|_| NipError::Nip52("Invalid start time".into()))?;
                    }
                }
                "end" => {
                    if let Some(v) = tag.get(1) {
                        end_time = v
                            .parse()
                            .map_err(|_| NipError::Nip52("Invalid end time".into()))?;
                    }
                }
                "start_tzid" => {
                    if let Some(v) = tag.get(1) {
                        start_timezone = Some(v.clone());
                    }
                }
                "end_tzid" => {
                    if let Some(v) = tag.get(1) {
                        end_timezone = Some(v.clone());
                    }
                }
                "location" => {
                    if let Some(v) = tag.get(1) {
                        locations.push(v.clone());
                    }
                }
                "g" => {
                    if let Some(v) = tag.get(1) {
                        geohash = Some(v.clone());
                    }
                }
                "r" => {
                    if let Some(v) = tag.get(1) {
                        references.push(v.clone());
                    }
                }
                "t" => {
                    if let Some(v) = tag.get(1) {
                        hashtags.push(v.clone());
                    }
                }
                "a" => {
                    let tag_obj = Tag::new(tag.clone());
                    if let Ok(reference) = CalendarReference::from_a_tag(&tag_obj) {
                        calendar_requests.push(reference);
                    }
                }
                "p" => {
                    if let Some(pubkey) = tag.get(1) {
                        let relay_url = tag.get(2).cloned();
                        participants.push(Participant {
                            pubkey: pubkey.clone(),
                            relay_url,
                            role: None,
                        });
                    }
                }
                "role" => {
                    // role tags reference a participant pubkey with a role
                    if tag.len() >= 3
                        && let Some(pos) = participants.iter().position(|p| p.pubkey == tag[1])
                    {
                        participants[pos].role = Some(tag[2].clone());
                    }
                }
                _ => {}
            }
        }

        if title.is_empty() {
            return Err(NipError::Nip52("Calendar event must have a title".into()));
        }

        Ok(CalendarEventMetadata {
            d_tag,
            title,
            summary,
            image,
            content: event.content.clone(),
            locations,
            geohash,
            references,
            hashtags,
            start_timezone,
            end_timezone,
            day_granularity,
            calendar_requests,
            event_type,
            start_time,
            end_time,
            participants,
        })
    }
}

// ==================== NIP-52 Calendar (kind:31924) ====================

/// NIP-52 Calendar metadata
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CalendarMetadata {
    /// d-tag: UUID for this calendar
    pub d_tag: String,
    /// Calendar title
    pub title: String,
    /// Calendar description (content field)
    pub description: String,
    /// Calendar owner pubkey
    pub owner_pubkey: String,
    /// Included event references
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_references: Vec<CalendarReference>,
    /// Calendar hashtags
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashtags: Vec<String>,
}

impl CalendarMetadata {
    pub fn new(
        d_tag: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        owner_pubkey: impl Into<String>,
    ) -> Self {
        Self {
            d_tag: d_tag.into(),
            title: title.into(),
            description: description.into(),
            owner_pubkey: owner_pubkey.into(),
            event_references: Vec::new(),
            hashtags: Vec::new(),
        }
    }

    pub fn with_event_reference(mut self, reference: CalendarReference) -> Self {
        self.event_references.push(reference);
        self
    }

    pub fn with_hashtag(mut self, hashtag: impl Into<String>) -> Self {
        self.hashtags.push(hashtag.into());
        self
    }
}

impl NipMetadata for CalendarMetadata {
    type Kind = Nip52Kind;

    fn kind(&self) -> Self::Kind {
        Nip52Kind::Calendar
    }

    fn validate(&self) -> NipResult<()> {
        if self.d_tag.is_empty() {
            return Err(NipError::Nip52("Calendar d-tag cannot be empty".into()));
        }
        if self.title.is_empty() {
            return Err(NipError::Nip52("Calendar title cannot be empty".into()));
        }
        if self.owner_pubkey.is_empty() {
            return Err(NipError::Nip52(
                "Calendar owner pubkey cannot be empty".into(),
            ));
        }
        // Validate pubkey format
        PubKey::new(&self.owner_pubkey)?;
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        let mut tags = vec![Tag::d(&self.d_tag)];

        // Title tag
        tags.push(Tag::new(vec![String::from("title"), self.title.clone()]));

        // Description tag (or use content field)
        tags.push(Tag::new(vec![
            String::from("description"),
            self.description.clone(),
        ]));

        // Event references (a-tags)
        for reference in &self.event_references {
            tags.push(reference.to_a_tag());
        }

        // Hashtag tags
        for hashtag in &self.hashtags {
            tags.push(Tag::t(hashtag));
        }

        tags
    }

    fn content(&self) -> String {
        self.description.clone()
    }

    fn d_tag(&self) -> Option<String> {
        Some(self.d_tag.clone())
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip52Kind::Calendar.kind_value() {
            return Err(NipError::Nip52(format!(
                "Expected kind 31924, got {}",
                event.kind
            )));
        }

        let d_tag = event
            .d_tag()
            .ok_or_else(|| NipError::Nip52("Calendar must have a d-tag".into()))?
            .to_string();

        let mut title = String::new();
        let mut description = event.content.clone();
        let mut event_references = Vec::new();
        let mut hashtags = Vec::new();
        let owner_pubkey = event.pubkey.clone();

        for tag in &event.tags {
            if tag.is_empty() {
                continue;
            }
            match tag[0].as_str() {
                "title" => {
                    if let Some(v) = tag.get(1) {
                        title = v.clone();
                    }
                }
                "description" => {
                    if let Some(v) = tag.get(1) {
                        description = v.clone();
                    }
                }
                "a" => {
                    let tag_obj = Tag::new(tag.clone());
                    if let Ok(reference) = CalendarReference::from_a_tag(&tag_obj) {
                        event_references.push(reference);
                    }
                }
                "t" => {
                    if let Some(v) = tag.get(1) {
                        hashtags.push(v.clone());
                    }
                }
                _ => {}
            }
        }

        if title.is_empty() {
            return Err(NipError::Nip52("Calendar must have a title".into()));
        }

        Ok(CalendarMetadata {
            d_tag,
            title,
            description,
            owner_pubkey,
            event_references,
            hashtags,
        })
    }
}

// ==================== NIP-52 RSVP (kind:31925) ====================

/// NIP-52 RSVP metadata
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RSVPMetadata {
    /// d-tag: unique identifier for this RSVP
    pub d_tag: String,
    /// Reference to the calendar event
    pub event_reference: CalendarReference,
    /// Optional specific event revision id (e-tag)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// RSVP status
    pub status: RSVPStatus,
    /// Free/busy indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_busy: Option<FreeBusyStatus>,
    /// Optional note (content field)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Public key of event author (for easy querying)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_author_pubkey: Option<String>,
}

impl RSVPMetadata {
    pub fn new(
        d_tag: impl Into<String>,
        event_reference: CalendarReference,
        status: RSVPStatus,
    ) -> Self {
        Self {
            d_tag: d_tag.into(),
            event_reference,
            event_id: None,
            status,
            free_busy: None,
            note: None,
            event_author_pubkey: None,
        }
    }

    pub fn with_event_id(mut self, event_id: impl Into<String>) -> Self {
        self.event_id = Some(event_id.into());
        self
    }

    pub fn with_free_busy(mut self, free_busy: FreeBusyStatus) -> Self {
        self.free_busy = Some(free_busy);
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn with_event_author(mut self, pubkey: impl Into<String>) -> Self {
        self.event_author_pubkey = Some(pubkey.into());
        self
    }
}

impl NipMetadata for RSVPMetadata {
    type Kind = Nip52Kind;

    fn kind(&self) -> Self::Kind {
        Nip52Kind::RSVP
    }

    fn validate(&self) -> NipResult<()> {
        if self.d_tag.is_empty() {
            return Err(NipError::Nip52("RSVP d-tag cannot be empty".into()));
        }
        // Validate event reference
        if self.event_reference.author_pubkey.is_empty() {
            return Err(NipError::Nip52(
                "Event reference author pubkey cannot be empty".into(),
            ));
        }
        if self.event_reference.d_identifier.is_empty() {
            return Err(NipError::Nip52(
                "Event reference d-identifier cannot be empty".into(),
            ));
        }
        Ok(())
    }

    fn to_tags(&self) -> Vec<Tag> {
        let mut tags = vec![Tag::d(&self.d_tag)];

        // Event reference (a-tag)
        tags.push(self.event_reference.to_a_tag());

        // Specific event ID (e-tag) if present
        if let Some(ref event_id) = self.event_id {
            tags.push(Tag::e(event_id));
        }

        // Event author pubkey (p-tag)
        if let Some(ref author) = self.event_author_pubkey {
            tags.push(Tag::p(author));
        }

        // Status tag (l-tag with namespace)
        tags.push(Tag::new(vec![
            String::from("l"),
            self.status.as_str().to_string(),
            String::from("status"),
        ]));

        // Free/busy tag (l-tag with namespace)
        if let Some(ref free_busy) = self.free_busy {
            tags.push(Tag::new(vec![
                String::from("l"),
                free_busy.as_str().to_string(),
                String::from("freebusy"),
            ]));
        }

        tags
    }

    fn content(&self) -> String {
        self.note.clone().unwrap_or_default()
    }

    fn d_tag(&self) -> Option<String> {
        Some(self.d_tag.clone())
    }

    fn from_raw_event(event: &RawNostrEvent) -> NipResult<Self> {
        if event.kind != Nip52Kind::RSVP.kind_value() {
            return Err(NipError::Nip52(format!(
                "Expected kind 31925, got {}",
                event.kind
            )));
        }

        let d_tag = event
            .d_tag()
            .ok_or_else(|| NipError::Nip52("RSVP must have a d-tag".into()))?
            .to_string();

        let mut event_reference = None;
        let mut event_id = None;
        let mut event_author_pubkey = None;
        let mut status = None;
        let mut free_busy = None;

        for tag in &event.tags {
            if tag.is_empty() {
                continue;
            }
            match tag[0].as_str() {
                "a" => {
                    let tag_obj = Tag::new(tag.clone());
                    if let Ok(reference) = CalendarReference::from_a_tag(&tag_obj) {
                        event_reference = Some(reference);
                    }
                }
                "e" => {
                    if let Some(v) = tag.get(1) {
                        event_id = Some(v.clone());
                    }
                }
                "p" => {
                    if let Some(v) = tag.get(1) {
                        event_author_pubkey = Some(v.clone());
                    }
                }
                "l" if tag.len() >= 3 => {
                    let value = tag[1].as_str();
                    let namespace = tag[2].as_str();
                    match namespace {
                        "status" => {
                            status = match value {
                                "accepted" => Some(RSVPStatus::Accepted),
                                "declined" => Some(RSVPStatus::Declined),
                                "tentative" => Some(RSVPStatus::Tentative),
                                _ => None,
                            };
                        }
                        "freebusy" => {
                            free_busy = match value {
                                "free" => Some(FreeBusyStatus::Free),
                                "busy" => Some(FreeBusyStatus::Busy),
                                _ => None,
                            };
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        let event_reference = event_reference
            .ok_or_else(|| NipError::Nip52("RSVP must reference a calendar event".into()))?;

        let status = status.ok_or_else(|| NipError::Nip52("RSVP must have a status".into()))?;

        Ok(RSVPMetadata {
            d_tag,
            event_reference,
            event_id,
            status,
            free_busy,
            note: if event.content.is_empty() {
                None
            } else {
                Some(event.content.clone())
            },
            event_author_pubkey,
        })
    }
}

#[cfg(feature = "nip44")]
impl super::nip44::NipMetadataEncrypt for CalendarEventMetadata {
    fn encrypted_content(&self) -> &str {
        &self.content
    }

    fn set_encrypted_content(&mut self, payload: String) {
        self.content = payload;
    }
}

// ==================== NIP-52 Helper Functions ====================

/// Generate a d-tag for a calendar event
pub fn generate_event_d_tag(title: &str, timestamp: u64) -> String {
    let hash = sha2::Sha256::digest(format!("{}:{}", title, timestamp).as_bytes());
    hex::encode(hash)[..16].to_string()
}

/// Generate a d-tag for an RSVP
pub fn generate_rsvp_d_tag(event_ref: &CalendarReference, responder_pubkey: &str) -> String {
    let hash = sha2::Sha256::digest(
        format!(
            "{}:{}:{}",
            event_ref.kind, event_ref.d_identifier, responder_pubkey
        )
        .as_bytes(),
    );
    hex::encode(hash)[..16].to_string()
}

/// Calculate the actual end time of an auction considering duration extensions
pub fn calculate_auction_end_time(start_time: u64, base_duration: u64, extensions: &[u64]) -> u64 {
    let total_extensions: u64 = extensions.iter().sum();
    start_time + base_duration + total_extensions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NipKind, NipMetadata, RawNostrEvent, Tag};

    const TEST_PUBKEY: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    const TEST_PUBKEY2: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    // ==================== Nip52Kind Tests ====================

    #[test]
    fn test_nip52_kind_kind_value() {
        assert_eq!(Nip52Kind::DateEvent.kind_value(), 31922);
        assert_eq!(Nip52Kind::TimeEvent.kind_value(), 31923);
        assert_eq!(Nip52Kind::Calendar.kind_value(), 31924);
        assert_eq!(Nip52Kind::RSVP.kind_value(), 31925);
    }

    // ==================== CalendarReference Tests ====================

    #[test]
    fn test_calendar_reference_new() {
        let ref_ = CalendarReference::new(31922, TEST_PUBKEY, "my-d-id");
        assert_eq!(ref_.kind, 31922);
        assert_eq!(ref_.author_pubkey, TEST_PUBKEY);
        assert_eq!(ref_.d_identifier, "my-d-id");
        assert!(ref_.relay_url.is_none());
    }

    #[test]
    fn test_calendar_reference_with_relay() {
        let ref_ = CalendarReference::new(31923, TEST_PUBKEY, "event-1")
            .with_relay("wss://relay.example.com");
        assert_eq!(ref_.kind, 31923);
        assert_eq!(ref_.author_pubkey, TEST_PUBKEY);
        assert_eq!(ref_.d_identifier, "event-1");
        assert_eq!(ref_.relay_url, Some("wss://relay.example.com".to_string()));
    }

    #[test]
    fn test_calendar_reference_to_a_tag_no_relay() {
        let ref_ = CalendarReference::new(31922, TEST_PUBKEY, "event-abc");
        let tag = ref_.to_a_tag();
        let inner: Vec<String> = tag.clone().into_inner();
        assert_eq!(inner[0], "a");
        assert_eq!(
            inner[1],
            "31922:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789:event-abc"
        );
        assert_eq!(inner.len(), 2);
    }

    #[test]
    fn test_calendar_reference_to_a_tag_with_relay() {
        let ref_ =
            CalendarReference::new(31923, TEST_PUBKEY, "event-xyz").with_relay("wss://relay.com");
        let tag = ref_.to_a_tag();
        let inner: Vec<String> = tag.clone().into_inner();
        assert_eq!(inner[0], "a");
        assert_eq!(
            inner[1],
            "31923:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789:event-xyz"
        );
        assert_eq!(inner[2], "wss://relay.com");
        assert_eq!(inner.len(), 3);
    }

    #[test]
    fn test_calendar_reference_from_a_tag_no_relay() {
        let tag = Tag::a(31922, TEST_PUBKEY, "my-event");
        let ref_ = CalendarReference::from_a_tag(&tag).unwrap();
        assert_eq!(ref_.kind, 31922);
        assert_eq!(ref_.author_pubkey, TEST_PUBKEY);
        assert_eq!(ref_.d_identifier, "my-event");
        assert!(ref_.relay_url.is_none());
    }

    #[test]
    fn test_calendar_reference_from_a_tag_with_relay() {
        let tag = Tag::a_with_relay(31923, TEST_PUBKEY, "evt-123", "wss://relay.example.com");
        let ref_ = CalendarReference::from_a_tag(&tag).unwrap();
        assert_eq!(ref_.kind, 31923);
        assert_eq!(ref_.author_pubkey, TEST_PUBKEY);
        assert_eq!(ref_.d_identifier, "evt-123");
        assert_eq!(ref_.relay_url, Some("wss://relay.example.com".to_string()));
    }

    #[test]
    fn test_calendar_reference_from_a_tag_invalid_format() {
        let tag = Tag::new(vec!["a".to_string()]);
        let result = CalendarReference::from_a_tag(&tag);
        assert!(result.is_err());
    }

    // ==================== RSVPStatus Tests ====================

    #[test]
    fn test_rsvp_status_as_str() {
        assert_eq!(RSVPStatus::Accepted.as_str(), "accepted");
        assert_eq!(RSVPStatus::Declined.as_str(), "declined");
        assert_eq!(RSVPStatus::Tentative.as_str(), "tentative");
    }

    // ==================== FreeBusyStatus Tests ====================

    #[test]
    fn test_free_busy_status_as_str() {
        assert_eq!(FreeBusyStatus::Free.as_str(), "free");
        assert_eq!(FreeBusyStatus::Busy.as_str(), "busy");
    }

    // ==================== EventType Tests ====================

    #[test]
    fn test_event_type_kind() {
        assert_eq!(EventType::TimeBased.kind(), Nip52Kind::TimeEvent);
        assert_eq!(EventType::DateBased.kind(), Nip52Kind::DateEvent);
    }

    // ==================== CalendarEventMetadata Tests ====================

    #[test]
    fn test_calendar_event_new_date_based() {
        let meta = CalendarEventMetadata::new_date_based("my-d", "My Event", "content", 1000, 2000);
        assert_eq!(meta.d_tag, "my-d");
        assert_eq!(meta.title, "My Event");
        assert_eq!(meta.content, "content");
        assert_eq!(meta.start_time, 1000);
        assert_eq!(meta.end_time, 2000);
        assert_eq!(meta.event_type, EventType::DateBased);
    }

    #[test]
    fn test_calendar_event_new_time_based() {
        let meta = CalendarEventMetadata::new_time_based("d-123", "Time Event", "desc", 100, 200);
        assert_eq!(meta.d_tag, "d-123");
        assert_eq!(meta.title, "Time Event");
        assert_eq!(meta.content, "desc");
        assert_eq!(meta.start_time, 100);
        assert_eq!(meta.end_time, 200);
        assert_eq!(meta.event_type, EventType::TimeBased);
    }

    #[test]
    fn test_calendar_event_builder_methods() {
        let cal_ref = CalendarReference::new(31924, TEST_PUBKEY, "cal-1");
        let meta = CalendarEventMetadata::new_date_based("d-1", "Event", "", 0, 10)
            .with_summary("A summary")
            .with_image("https://example.com/img.png")
            .with_location("New York")
            .with_location("London")
            .with_geohash("9q8yy")
            .with_reference("https://example.com")
            .with_hashtag("meeting")
            .with_hashtag("work")
            .with_calendar_request(cal_ref)
            .with_participant(TEST_PUBKEY2, Some("organizer".to_string()))
            .with_participant(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                None,
            );

        assert_eq!(meta.summary, Some("A summary".to_string()));
        assert_eq!(meta.image, Some("https://example.com/img.png".to_string()));
        assert_eq!(meta.locations, vec!["New York", "London"]);
        assert_eq!(meta.geohash, Some("9q8yy".to_string()));
        assert_eq!(meta.references, vec!["https://example.com"]);
        assert_eq!(meta.hashtags, vec!["meeting", "work"]);
        assert_eq!(meta.calendar_requests.len(), 1);
        assert_eq!(meta.participants.len(), 2);
        assert_eq!(meta.participants[0].pubkey, TEST_PUBKEY2);
        assert_eq!(meta.participants[0].role, Some("organizer".to_string()));
        assert!(meta.participants[0].relay_url.is_none());
        assert!(meta.participants[1].role.is_none());
    }

    #[test]
    fn test_calendar_event_validate_passes() {
        let meta = CalendarEventMetadata::new_time_based("d-ok", "Valid Event", "", 100, 200);
        assert!(meta.validate().is_ok());
    }

    #[test]
    fn test_calendar_event_validate_empty_d_tag() {
        let meta = CalendarEventMetadata::new_time_based("", "Event", "", 100, 200);
        assert!(meta.validate().is_err());
    }

    #[test]
    fn test_calendar_event_validate_empty_title() {
        let meta = CalendarEventMetadata::new_time_based("d-1", "", "", 100, 200);
        assert!(meta.validate().is_err());
    }

    #[test]
    fn test_calendar_event_validate_start_gt_end() {
        let meta = CalendarEventMetadata::new_time_based("d-1", "Event", "", 200, 100);
        assert!(meta.validate().is_err());
    }

    #[test]
    fn test_calendar_event_to_tags() {
        let cal_ref = CalendarReference::new(31924, TEST_PUBKEY, "cal-abc");
        let meta = CalendarEventMetadata::new_time_based(
            "my-id",
            "Test Event",
            "some content",
            1000,
            2000,
        )
        .with_summary("Short summary")
        .with_image("https://img.url")
        .with_location("Berlin")
        .with_geohash("u33d")
        .with_reference("https://ref.com")
        .with_hashtag("conference")
        .with_calendar_request(cal_ref)
        .with_participant(TEST_PUBKEY2, Some("speaker".to_string()));

        let tags = meta.to_tags();

        // Check d tag
        assert!(tags.iter().any(|t| t.as_slice() == ["d", "my-id"]));

        // Check title tag
        assert!(tags.iter().any(|t| t.as_slice() == ["title", "Test Event"]));

        // Check summary tag
        assert!(
            tags.iter()
                .any(|t| t.as_slice() == ["summary", "Short summary"])
        );

        // Check image tag
        assert!(
            tags.iter()
                .any(|t| t.as_slice() == ["image", "https://img.url"])
        );

        // Check start/end
        assert!(tags.iter().any(|t| t.as_slice() == ["start", "1000"]));
        assert!(tags.iter().any(|t| t.as_slice() == ["end", "2000"]));

        // Check location
        assert!(tags.iter().any(|t| t.as_slice() == ["location", "Berlin"]));

        // Check geohash
        assert!(tags.iter().any(|t| t.as_slice() == ["g", "u33d"]));

        // Check reference
        assert!(
            tags.iter()
                .any(|t| t.as_slice() == ["r", "https://ref.com"])
        );

        // Check hashtag
        assert!(tags.iter().any(|t| t.as_slice() == ["t", "conference"]));

        // Check a-tag for calendar request
        let expected_a = format!("31924:{}:cal-abc", TEST_PUBKEY);
        assert!(tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "a" && s[1] == expected_a
        }));

        // Check participant p-tag
        assert!(tags.iter().any(|t| t.as_slice() == ["p", TEST_PUBKEY2]));

        // Check role tag
        assert!(
            tags.iter()
                .any(|t| t.as_slice() == ["role", TEST_PUBKEY2, "speaker"])
        );
    }

    #[test]
    fn test_calendar_event_d_tag() {
        let meta = CalendarEventMetadata::new_date_based("my-d-tag", "Title", "", 0, 1);
        assert_eq!(meta.d_tag(), Some("my-d-tag".to_string()));
    }

    #[test]
    fn test_calendar_event_content() {
        let meta = CalendarEventMetadata::new_date_based("d", "Title", "hello world", 0, 1);
        assert_eq!(meta.content(), "hello world");
    }

    #[test]
    fn test_calendar_event_kind() {
        let date_based = CalendarEventMetadata::new_date_based("d", "T", "", 0, 1);
        let time_based = CalendarEventMetadata::new_time_based("d", "T", "", 0, 1);
        assert_eq!(Nip52Kind::DateEvent, date_based.kind());
        assert_eq!(Nip52Kind::TimeEvent, time_based.kind());
    }

    #[test]
    fn test_calendar_event_from_raw_event_date_based() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 0,
            kind: 31922,
            tags: vec![
                vec!["d".to_string(), "evt-001".to_string()],
                vec!["title".to_string(), "Date Event".to_string()],
                vec!["start".to_string(), "1000".to_string()],
                vec!["end".to_string(), "2000".to_string()],
            ],
            content: "event content".to_string(),
            sig: String::new(),
        };

        let meta = CalendarEventMetadata::from_raw_event(&event).unwrap();
        assert_eq!(meta.d_tag, "evt-001");
        assert_eq!(meta.title, "Date Event");
        assert_eq!(meta.event_type, EventType::DateBased);
        assert_eq!(meta.start_time, 1000);
        assert_eq!(meta.end_time, 2000);
        assert_eq!(meta.content, "event content");
    }

    #[test]
    fn test_calendar_event_from_raw_event_time_based() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 0,
            kind: 31923,
            tags: vec![
                vec!["d".to_string(), "evt-002".to_string()],
                vec!["title".to_string(), "Time Event".to_string()],
                vec!["start".to_string(), "500".to_string()],
                vec!["end".to_string(), "1500".to_string()],
            ],
            content: String::new(),
            sig: String::new(),
        };

        let meta = CalendarEventMetadata::from_raw_event(&event).unwrap();
        assert_eq!(meta.d_tag, "evt-002");
        assert_eq!(meta.title, "Time Event");
        assert_eq!(meta.event_type, EventType::TimeBased);
        assert_eq!(meta.start_time, 500);
        assert_eq!(meta.end_time, 1500);
    }

    #[test]
    fn test_calendar_event_from_raw_event_wrong_kind() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: String::new(),
            created_at: 0,
            kind: 9999,
            tags: vec![],
            content: String::new(),
            sig: String::new(),
        };

        let result = CalendarEventMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn test_calendar_event_from_raw_event_missing_d_tag() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: String::new(),
            created_at: 0,
            kind: 31922,
            tags: vec![vec!["title".to_string(), "No D Tag".to_string()]],
            content: String::new(),
            sig: String::new(),
        };

        let result = CalendarEventMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn test_calendar_event_from_raw_event_missing_title() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: String::new(),
            created_at: 0,
            kind: 31922,
            tags: vec![vec!["d".to_string(), "no-title".to_string()]],
            content: String::new(),
            sig: String::new(),
        };

        let result = CalendarEventMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn test_calendar_event_from_raw_event_with_all_tags() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 0,
            kind: 31923,
            tags: vec![
                vec!["d".to_string(), "full-event".to_string()],
                vec!["title".to_string(), "Full Event".to_string()],
                vec!["summary".to_string(), "A full test".to_string()],
                vec!["image".to_string(), "https://img.url".to_string()],
                vec!["start".to_string(), "100".to_string()],
                vec!["end".to_string(), "200".to_string()],
                vec!["start_tzid".to_string(), "UTC".to_string()],
                vec!["end_tzid".to_string(), "UTC".to_string()],
                vec!["location".to_string(), "NYC".to_string()],
                vec!["g".to_string(), "9q8yy".to_string()],
                vec!["r".to_string(), "https://ref.co".to_string()],
                vec!["t".to_string(), "test".to_string()],
                vec!["a".to_string(), format!("31924:{}:cal-ref", TEST_PUBKEY)],
                vec!["p".to_string(), TEST_PUBKEY2.to_string()],
                vec![
                    "role".to_string(),
                    TEST_PUBKEY2.to_string(),
                    "host".to_string(),
                ],
            ],
            content: "full content".to_string(),
            sig: String::new(),
        };

        let meta = CalendarEventMetadata::from_raw_event(&event).unwrap();
        assert_eq!(meta.d_tag, "full-event");
        assert_eq!(meta.title, "Full Event");
        assert_eq!(meta.summary, Some("A full test".to_string()));
        assert_eq!(meta.image, Some("https://img.url".to_string()));
        assert_eq!(meta.start_time, 100);
        assert_eq!(meta.end_time, 200);
        assert_eq!(meta.start_timezone, Some("UTC".to_string()));
        assert_eq!(meta.end_timezone, Some("UTC".to_string()));
        assert_eq!(meta.locations, vec!["NYC"]);
        assert_eq!(meta.geohash, Some("9q8yy".to_string()));
        assert_eq!(meta.references, vec!["https://ref.co"]);
        assert_eq!(meta.hashtags, vec!["test"]);
        assert_eq!(meta.calendar_requests.len(), 1);
        assert_eq!(meta.calendar_requests[0].d_identifier, "cal-ref");
        assert_eq!(meta.participants.len(), 1);
        assert_eq!(meta.participants[0].pubkey, TEST_PUBKEY2);
        assert_eq!(meta.participants[0].role, Some("host".to_string()));
        assert_eq!(meta.content, "full content");
    }

    #[test]
    fn test_calendar_event_from_raw_event_empty_tags_skipped() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 0,
            kind: 31922,
            tags: vec![
                vec![], // empty tag - should be skipped
                vec!["d".to_string(), "skip-test".to_string()],
                vec!["title".to_string(), "Skip Test".to_string()],
                vec!["start".to_string(), "0".to_string()],
                vec!["end".to_string(), "1".to_string()],
            ],
            content: String::new(),
            sig: String::new(),
        };

        let meta = CalendarEventMetadata::from_raw_event(&event).unwrap();
        assert_eq!(meta.d_tag, "skip-test");
    }

    // ==================== CalendarMetadata Tests ====================

    #[test]
    fn test_calendar_metadata_new() {
        let meta = CalendarMetadata::new("cal-d", "My Calendar", "Description", TEST_PUBKEY);
        assert_eq!(meta.d_tag, "cal-d");
        assert_eq!(meta.title, "My Calendar");
        assert_eq!(meta.description, "Description");
        assert_eq!(meta.owner_pubkey, TEST_PUBKEY);
        assert!(meta.event_references.is_empty());
        assert!(meta.hashtags.is_empty());
    }

    #[test]
    fn test_calendar_metadata_builder() {
        let ref1 = CalendarReference::new(31922, TEST_PUBKEY, "evt-1");
        let ref2 = CalendarReference::new(31923, TEST_PUBKEY2, "evt-2");
        let meta = CalendarMetadata::new("cal-1", "My Cal", "desc", TEST_PUBKEY)
            .with_event_reference(ref1)
            .with_event_reference(ref2)
            .with_hashtag("sports")
            .with_hashtag("news");

        assert_eq!(meta.event_references.len(), 2);
        assert_eq!(meta.hashtags, vec!["sports", "news"]);
    }

    #[test]
    fn test_calendar_metadata_validate_passes() {
        let meta = CalendarMetadata::new("valid-cal", "Valid Calendar", "desc", TEST_PUBKEY);
        assert!(meta.validate().is_ok());
    }

    #[test]
    fn test_calendar_metadata_validate_empty_d_tag() {
        let meta = CalendarMetadata::new("", "Calendar", "desc", TEST_PUBKEY);
        assert!(meta.validate().is_err());
    }

    #[test]
    fn test_calendar_metadata_validate_empty_title() {
        let meta = CalendarMetadata::new("d-1", "", "desc", TEST_PUBKEY);
        assert!(meta.validate().is_err());
    }

    #[test]
    fn test_calendar_metadata_to_tags() {
        let ref1 = CalendarReference::new(31922, TEST_PUBKEY, "event-abc");
        let meta = CalendarMetadata::new("cal-id", "My Calendar", "A description", TEST_PUBKEY)
            .with_event_reference(ref1)
            .with_hashtag("important");

        let tags = meta.to_tags();

        // Check d tag
        assert!(tags.iter().any(|t| t.as_slice() == ["d", "cal-id"]));

        // Check title tag
        assert!(
            tags.iter()
                .any(|t| t.as_slice() == ["title", "My Calendar"])
        );

        // Check description tag
        assert!(
            tags.iter()
                .any(|t| t.as_slice() == ["description", "A description"])
        );

        // Check a-tag for event reference
        let expected_a = format!("31922:{}:event-abc", TEST_PUBKEY);
        assert!(tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "a" && s[1] == expected_a
        }));

        // Check hashtag
        assert!(tags.iter().any(|t| t.as_slice() == ["t", "important"]));
    }

    #[test]
    fn test_calendar_metadata_content() {
        let meta = CalendarMetadata::new("d", "Title", "my description", TEST_PUBKEY);
        assert_eq!(meta.content(), "my description");
    }

    #[test]
    fn test_calendar_metadata_d_tag() {
        let meta = CalendarMetadata::new("my-d-tag", "Title", "desc", TEST_PUBKEY);
        assert_eq!(meta.d_tag(), Some("my-d-tag".to_string()));
    }

    #[test]
    fn test_calendar_metadata_kind() {
        let meta = CalendarMetadata::new("d", "T", "desc", TEST_PUBKEY);
        assert_eq!(meta.kind(), Nip52Kind::Calendar);
    }

    #[test]
    fn test_calendar_metadata_from_raw_event() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 0,
            kind: 31924,
            tags: vec![
                vec!["d".to_string(), "cal-001".to_string()],
                vec!["title".to_string(), "Test Calendar".to_string()],
                vec!["description".to_string(), "A test calendar".to_string()],
                vec!["a".to_string(), format!("31922:{}:evt-ref", TEST_PUBKEY)],
                vec!["t".to_string(), "calendar".to_string()],
            ],
            content: "A test calendar".to_string(),
            sig: String::new(),
        };

        let meta = CalendarMetadata::from_raw_event(&event).unwrap();
        assert_eq!(meta.d_tag, "cal-001");
        assert_eq!(meta.title, "Test Calendar");
        assert_eq!(meta.description, "A test calendar");
        assert_eq!(meta.owner_pubkey, TEST_PUBKEY);
        assert_eq!(meta.event_references.len(), 1);
        assert_eq!(meta.event_references[0].d_identifier, "evt-ref");
        assert_eq!(meta.hashtags, vec!["calendar"]);
    }

    #[test]
    fn test_calendar_metadata_from_raw_event_wrong_kind() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: String::new(),
            created_at: 0,
            kind: 31922,
            tags: vec![],
            content: String::new(),
            sig: String::new(),
        };

        let result = CalendarMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn test_calendar_metadata_from_raw_event_missing_d_tag() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 0,
            kind: 31924,
            tags: vec![vec!["title".to_string(), "Calendar".to_string()]],
            content: String::new(),
            sig: String::new(),
        };

        let result = CalendarMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn test_calendar_metadata_from_raw_event_missing_title() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 0,
            kind: 31924,
            tags: vec![vec!["d".to_string(), "no-title".to_string()]],
            content: String::new(),
            sig: String::new(),
        };

        let result = CalendarMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    // ==================== RSVPMetadata Tests ====================

    fn make_event_ref() -> CalendarReference {
        CalendarReference::new(31923, TEST_PUBKEY, "event-d-id")
    }

    fn make_event_ref_with_relay() -> CalendarReference {
        CalendarReference::new(31923, TEST_PUBKEY, "event-d-id")
            .with_relay("wss://relay.example.com")
    }

    #[test]
    fn test_rsvp_metadata_new() {
        let event_ref = make_event_ref();
        let rsvp = RSVPMetadata::new("rsvp-d", event_ref.clone(), RSVPStatus::Accepted);
        assert_eq!(rsvp.d_tag, "rsvp-d");
        assert_eq!(rsvp.event_reference.kind, 31923);
        assert_eq!(rsvp.event_reference.author_pubkey, TEST_PUBKEY);
        assert_eq!(rsvp.event_reference.d_identifier, "event-d-id");
        assert_eq!(rsvp.status, RSVPStatus::Accepted);
        assert!(rsvp.event_id.is_none());
        assert!(rsvp.free_busy.is_none());
        assert!(rsvp.note.is_none());
        assert!(rsvp.event_author_pubkey.is_none());
    }

    #[test]
    fn test_rsvp_metadata_builder() {
        let event_ref = make_event_ref();
        let rsvp = RSVPMetadata::new("rsvp-id", event_ref, RSVPStatus::Tentative)
            .with_event_id("event-abc-123")
            .with_free_busy(FreeBusyStatus::Busy)
            .with_note("Maybe attending")
            .with_event_author(TEST_PUBKEY2);

        assert_eq!(rsvp.event_id, Some("event-abc-123".to_string()));
        assert_eq!(rsvp.free_busy, Some(FreeBusyStatus::Busy));
        assert_eq!(rsvp.note, Some("Maybe attending".to_string()));
        assert_eq!(rsvp.event_author_pubkey, Some(TEST_PUBKEY2.to_string()));
    }

    #[test]
    fn test_rsvp_metadata_validate_passes() {
        let event_ref = make_event_ref();
        let rsvp = RSVPMetadata::new("d-ok", event_ref, RSVPStatus::Accepted);
        assert!(rsvp.validate().is_ok());
    }

    #[test]
    fn test_rsvp_metadata_validate_empty_d_tag() {
        let event_ref = make_event_ref();
        let rsvp = RSVPMetadata::new("", event_ref, RSVPStatus::Accepted);
        assert!(rsvp.validate().is_err());
    }

    #[test]
    fn test_rsvp_metadata_validate_empty_author_pubkey() {
        let event_ref = CalendarReference::new(31923, "", "event-id");
        let rsvp = RSVPMetadata::new("d-1", event_ref, RSVPStatus::Accepted);
        assert!(rsvp.validate().is_err());
    }

    #[test]
    fn test_rsvp_metadata_validate_empty_d_identifier() {
        let event_ref = CalendarReference::new(31923, TEST_PUBKEY, "");
        let rsvp = RSVPMetadata::new("d-1", event_ref, RSVPStatus::Accepted);
        assert!(rsvp.validate().is_err());
    }

    #[test]
    fn test_rsvp_metadata_to_tags() {
        let event_ref = make_event_ref_with_relay();
        let rsvp = RSVPMetadata::new("my-rsvp", event_ref, RSVPStatus::Declined)
            .with_event_id("evt-id-001")
            .with_free_busy(FreeBusyStatus::Free)
            .with_event_author(TEST_PUBKEY2);

        let tags = rsvp.to_tags();

        // Check d tag
        assert!(tags.iter().any(|t| t.as_slice() == ["d", "my-rsvp"]));

        // Check a tag (event reference with relay)
        let expected_a = format!("31923:{}:event-d-id", TEST_PUBKEY);
        assert!(tags.iter().any(|t| {
            let s = t.as_slice();
            s.len() >= 2 && s[0] == "a" && s[1] == expected_a
        }));

        // Check e tag
        assert!(tags.iter().any(|t| t.as_slice() == ["e", "evt-id-001"]));

        // Check p tag (event author)
        assert!(tags.iter().any(|t| t.as_slice() == ["p", TEST_PUBKEY2]));

        // Check status l-tag
        assert!(
            tags.iter()
                .any(|t| t.as_slice() == ["l", "declined", "status"])
        );

        // Check freebusy l-tag
        assert!(
            tags.iter()
                .any(|t| t.as_slice() == ["l", "free", "freebusy"])
        );
    }

    #[test]
    fn test_rsvp_metadata_content() {
        let event_ref = make_event_ref();
        // with note
        let rsvp =
            RSVPMetadata::new("d", event_ref.clone(), RSVPStatus::Accepted).with_note("My note");
        assert_eq!(rsvp.content(), "My note");

        // without note
        let rsvp2 = RSVPMetadata::new("d", event_ref, RSVPStatus::Accepted);
        assert_eq!(rsvp2.content(), "");
    }

    #[test]
    fn test_rsvp_metadata_d_tag() {
        let event_ref = make_event_ref();
        let rsvp = RSVPMetadata::new("my-rsvp-d", event_ref, RSVPStatus::Accepted);
        assert_eq!(rsvp.d_tag(), Some("my-rsvp-d".to_string()));
    }

    #[test]
    fn test_rsvp_metadata_kind() {
        let event_ref = make_event_ref();
        let rsvp = RSVPMetadata::new("d", event_ref, RSVPStatus::Accepted);
        assert_eq!(rsvp.kind(), Nip52Kind::RSVP);
    }

    #[test]
    fn test_rsvp_metadata_from_raw_event_accepted() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 0,
            kind: 31925,
            tags: vec![
                vec!["d".to_string(), "rsvp-001".to_string()],
                vec!["a".to_string(), format!("31923:{}:event-abc", TEST_PUBKEY)],
                vec!["e".to_string(), "event-id-123".to_string()],
                vec!["p".to_string(), TEST_PUBKEY2.to_string()],
                vec![
                    "l".to_string(),
                    "accepted".to_string(),
                    "status".to_string(),
                ],
            ],
            content: "See you there!".to_string(),
            sig: String::new(),
        };

        let rsvp = RSVPMetadata::from_raw_event(&event).unwrap();
        assert_eq!(rsvp.d_tag, "rsvp-001");
        assert_eq!(rsvp.event_reference.d_identifier, "event-abc");
        assert_eq!(rsvp.status, RSVPStatus::Accepted);
        assert_eq!(rsvp.event_id, Some("event-id-123".to_string()));
        assert_eq!(rsvp.event_author_pubkey, Some(TEST_PUBKEY2.to_string()));
        assert_eq!(rsvp.note, Some("See you there!".to_string()));
    }

    #[test]
    fn test_rsvp_metadata_from_raw_event_tentative() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: TEST_PUBKEY.to_string(),
            created_at: 0,
            kind: 31925,
            tags: vec![
                vec!["d".to_string(), "rsvp-tent".to_string()],
                vec!["a".to_string(), format!("31923:{}:event-xyz", TEST_PUBKEY)],
                vec![
                    "l".to_string(),
                    "tentative".to_string(),
                    "status".to_string(),
                ],
                vec!["l".to_string(), "busy".to_string(), "freebusy".to_string()],
            ],
            content: String::new(),
            sig: String::new(),
        };

        let rsvp = RSVPMetadata::from_raw_event(&event).unwrap();
        assert_eq!(rsvp.d_tag, "rsvp-tent");
        assert_eq!(rsvp.status, RSVPStatus::Tentative);
        assert_eq!(rsvp.free_busy, Some(FreeBusyStatus::Busy));
        assert!(rsvp.note.is_none());
    }

    #[test]
    fn test_rsvp_metadata_from_raw_event_wrong_kind() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: String::new(),
            created_at: 0,
            kind: 31922,
            tags: vec![],
            content: String::new(),
            sig: String::new(),
        };

        let result = RSVPMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn test_rsvp_metadata_from_raw_event_missing_d_tag() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: String::new(),
            created_at: 0,
            kind: 31925,
            tags: vec![vec!["a".to_string(), format!("31923:{}:e", TEST_PUBKEY)]],
            content: String::new(),
            sig: String::new(),
        };

        let result = RSVPMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn test_rsvp_metadata_from_raw_event_missing_event_reference() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: String::new(),
            created_at: 0,
            kind: 31925,
            tags: vec![vec!["d".to_string(), "rsvp-no-ref".to_string()]],
            content: String::new(),
            sig: String::new(),
        };

        let result = RSVPMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn test_rsvp_metadata_from_raw_event_missing_status() {
        let event = RawNostrEvent {
            id: String::new(),
            pubkey: String::new(),
            created_at: 0,
            kind: 31925,
            tags: vec![
                vec!["d".to_string(), "rsvp-no-status".to_string()],
                vec!["a".to_string(), format!("31923:{}:e", TEST_PUBKEY)],
            ],
            content: String::new(),
            sig: String::new(),
        };

        let result = RSVPMetadata::from_raw_event(&event);
        assert!(result.is_err());
    }

    // ==================== Helper Function Tests ====================

    #[test]
    fn test_generate_event_d_tag() {
        let d_tag = generate_event_d_tag("My Event", 1234567890);
        assert_eq!(d_tag.len(), 16);
        // Verify it's hex
        assert!(d_tag.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_event_d_tag_deterministic() {
        let d_tag1 = generate_event_d_tag("Same Event", 1000);
        let d_tag2 = generate_event_d_tag("Same Event", 1000);
        assert_eq!(d_tag1, d_tag2);
    }

    #[test]
    fn test_generate_event_d_tag_different_inputs() {
        let d_tag1 = generate_event_d_tag("Event A", 1000);
        let d_tag2 = generate_event_d_tag("Event B", 1000);
        assert_ne!(d_tag1, d_tag2);
    }

    #[test]
    fn test_generate_rsvp_d_tag() {
        let event_ref = CalendarReference::new(31922, TEST_PUBKEY, "event-id");
        let d_tag = generate_rsvp_d_tag(&event_ref, TEST_PUBKEY2);
        assert_eq!(d_tag.len(), 16);
        assert!(d_tag.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_rsvp_d_tag_deterministic() {
        let event_ref = CalendarReference::new(31922, TEST_PUBKEY, "event-id");
        let d_tag1 = generate_rsvp_d_tag(&event_ref, TEST_PUBKEY2);
        let d_tag2 = generate_rsvp_d_tag(&event_ref, TEST_PUBKEY2);
        assert_eq!(d_tag1, d_tag2);
    }

    #[test]
    fn test_calculate_auction_end_time_no_extensions() {
        let end = calculate_auction_end_time(1000, 3600, &[]);
        assert_eq!(end, 4600);
    }

    #[test]
    fn test_calculate_auction_end_time_with_extensions() {
        let end = calculate_auction_end_time(1000, 3600, &[60, 120, 30]);
        assert_eq!(end, 4810); // 1000 + 3600 + 60 + 120 + 30
    }

    #[test]
    fn test_calculate_auction_end_time_single_extension() {
        let end = calculate_auction_end_time(500, 300, &[100]);
        assert_eq!(end, 900);
    }
}
