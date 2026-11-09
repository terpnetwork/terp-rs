use crate::types::Tag;

/// Builder for constructing tags with a fluent API
#[derive(Debug, Clone, Default)]
pub struct TagBuilder {
    tags: Vec<Tag>,
}

impl TagBuilder {
    pub fn tag(mut self, tag: Tag) -> Self {
        self.tags.push(tag);
        self
    }

    pub fn extend(mut self, tags: Vec<Tag>) -> Self {
        self.tags.extend(tags);
        self
    }

    // NIP-73 convenience
    pub fn external_id(mut self, id: impl Into<String>, k: impl Into<String>) -> Self {
        let (i_tag, k_tag) = Tag::i(id, k);
        self.tags.push(i_tag);
        self.tags.push(k_tag);
        self
    }

    pub fn external_id_with_hint(
        mut self,
        id: impl Into<String>,
        k: impl Into<String>,
        url_hint: Option<impl Into<String>>,
    ) -> Self {
        let mut extra = Tag::i_with_hint(id, k, url_hint.map(Into::into));
        self.tags.append(&mut extra);
        self
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn e(mut self, event_id: impl Into<String>) -> Self {
        self.tags.push(Tag::e(event_id));
        self
    }

    pub fn e_with_relay(mut self, event_id: impl Into<String>, relay: impl Into<String>) -> Self {
        self.tags.push(Tag::e_with_relay(event_id, relay));
        self
    }

    pub fn e_full(
        mut self,
        event_id: impl Into<String>,
        relay: impl Into<String>,
        author: impl Into<String>,
    ) -> Self {
        self.tags.push(Tag::e_full(event_id, relay, author));
        self
    }

    pub fn p(mut self, pubkey: impl Into<String>) -> Self {
        self.tags.push(Tag::p(pubkey));
        self
    }

    pub fn p_with_relay(mut self, pubkey: impl Into<String>, relay: impl Into<String>) -> Self {
        self.tags.push(Tag::p_with_relay(pubkey, relay));
        self
    }

    pub fn a(mut self, kind: u16, pubkey: impl Into<String>, d: impl Into<String>) -> Self {
        self.tags.push(Tag::a(kind, pubkey, d));
        self
    }

    pub fn a_with_relay(
        mut self,
        kind: u16,
        pubkey: impl Into<String>,
        d: impl Into<String>,
        relay: impl Into<String>,
    ) -> Self {
        self.tags.push(Tag::a_with_relay(kind, pubkey, d, relay));
        self
    }

    pub fn d(mut self, value: impl Into<String>) -> Self {
        self.tags.push(Tag::d(value));
        self
    }

    pub fn t(mut self, value: impl Into<String>) -> Self {
        self.tags.push(Tag::t(value));
        self
    }

    pub fn g(mut self, value: impl Into<String>) -> Self {
        self.tags.push(Tag::g(value));
        self
    }

    pub fn r(mut self, value: impl Into<String>) -> Self {
        self.tags.push(Tag::r(value));
        self
    }

    pub fn custom(mut self, tag: Tag) -> Self {
        self.tags.push(tag);
        self
    }

    pub fn build(self) -> Vec<Tag> {
        self.tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_empty_builder() {
        let builder = TagBuilder::new();
        let tags = builder.build();
        assert_eq!(tags.len(), 0);
    }

    #[test]
    fn test_default_creates_empty_builder() {
        let builder = TagBuilder::default();
        let tags = builder.build();
        assert_eq!(tags.len(), 0);
    }

    #[test]
    fn test_e_tag() {
        let tags = TagBuilder::new().e("event123").build();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], Tag::e("event123"));
    }

    #[test]
    fn test_e_with_relay_tag() {
        let tags = TagBuilder::new()
            .e_with_relay("event123", "wss://relay.example.com")
            .build();
        assert_eq!(tags.len(), 1);
        assert_eq!(
            tags[0],
            Tag::e_with_relay("event123", "wss://relay.example.com")
        );
    }

    #[test]
    fn test_e_full_tag() {
        let tags = TagBuilder::new()
            .e_full("event123", "wss://relay.example.com", "pubkey123")
            .build();
        assert_eq!(tags.len(), 1);
        assert_eq!(
            tags[0],
            Tag::e_full("event123", "wss://relay.example.com", "pubkey123")
        );
    }

    #[test]
    fn test_p_tag() {
        let tags = TagBuilder::new().p("pubkey123").build();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], Tag::p("pubkey123"));
    }

    #[test]
    fn test_p_with_relay_tag() {
        let tags = TagBuilder::new()
            .p_with_relay("pubkey123", "wss://relay.example.com")
            .build();
        assert_eq!(tags.len(), 1);
        assert_eq!(
            tags[0],
            Tag::p_with_relay("pubkey123", "wss://relay.example.com")
        );
    }

    #[test]
    fn test_a_tag() {
        let tags = TagBuilder::new().a(30023, "pubkey123", "dvalue").build();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], Tag::a(30023, "pubkey123", "dvalue"));
    }

    #[test]
    fn test_a_with_relay_tag() {
        let tags = TagBuilder::new()
            .a_with_relay(30023, "pubkey123", "dvalue", "wss://relay.example.com")
            .build();
        assert_eq!(tags.len(), 1);
        assert_eq!(
            tags[0],
            Tag::a_with_relay(30023, "pubkey123", "dvalue", "wss://relay.example.com",)
        );
    }

    #[test]
    fn test_d_tag() {
        let tags = TagBuilder::new().d("my-identifier").build();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], Tag::d("my-identifier"));
    }

    #[test]
    fn test_t_tag() {
        let tags = TagBuilder::new().t("nostr").build();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], Tag::t("nostr"));
    }

    #[test]
    fn test_g_tag() {
        let tags = TagBuilder::new().g("9q8yyz").build();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], Tag::g("9q8yyz"));
    }

    #[test]
    fn test_r_tag() {
        let tags = TagBuilder::new().r("https://example.com").build();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], Tag::r("https://example.com"));
    }

    #[test]
    fn test_custom_tag() {
        let custom = Tag::new(vec![
            "my".to_string(),
            "custom".to_string(),
            "tag".to_string(),
        ]);
        let tags = TagBuilder::new().custom(custom.clone()).build();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0], custom);
    }

    #[test]
    fn test_fluent_chaining() {
        let tags = TagBuilder::new()
            .e("event1")
            .p("pubkey1")
            .t("nostr")
            .build();
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0], Tag::e("event1"));
        assert_eq!(tags[1], Tag::p("pubkey1"));
        assert_eq!(tags[2], Tag::t("nostr"));
    }

    #[test]
    fn test_build_returns_correct_number_of_tags() {
        let tags = TagBuilder::new()
            .e("e1")
            .e_with_relay("e2", "relay1")
            .e_full("e3", "relay2", "author1")
            .p("p1")
            .p_with_relay("p2", "relay3")
            .a(1, "a-pubkey", "a-d")
            .a_with_relay(2, "a2-pubkey", "a2-d", "relay4")
            .d("d-value")
            .t("hashtag")
            .g("geohash")
            .r("reference")
            .custom(Tag::new(vec!["x".to_string()]))
            .build();
        assert_eq!(tags.len(), 12);
    }
}
