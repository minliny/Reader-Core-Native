//! Reader-Core RSS — feed parsing and subscription state.

use std::collections::{HashMap, HashSet};

/// Parsed RSS/Atom feed metadata plus entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssFeed {
    pub title: String,
    pub feed_url: Option<String>,
    pub site_url: Option<String>,
    pub description: Option<String>,
    pub entries: Vec<RssEntry>,
}

/// One item from an RSS channel or Atom feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssEntry {
    /// Stable entry identity, derived from `guid`/`id`/`link`/`title`.
    pub id: String,
    pub title: String,
    pub link: Option<String>,
    pub summary: Option<String>,
    /// Raw date string from `pubDate`, `updated`, or `published`.
    pub published_at: Option<String>,
}

/// Stored subscription state for a feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssSubscription {
    pub subscription_id: String,
    pub feed_url: String,
    pub title: String,
    pub site_url: Option<String>,
    pub enabled: bool,
    pub last_fetch_at: Option<i64>,
    pub last_entry_id: Option<String>,
    pub unread_count: u32,
}

/// Result of merging a newly fetched feed into subscription state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssRefreshResult {
    pub subscription: RssSubscription,
    pub new_entries: Vec<RssEntry>,
}

/// Stored state for one RSS entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssEntryState {
    pub subscription_id: String,
    pub entry: RssEntry,
    pub first_seen_at: i64,
    pub read: bool,
    pub read_at: Option<i64>,
    pub starred: bool,
}

/// In-memory RSS subscription and entry state.
///
/// This is a data-layer state machine. It deliberately does not fetch network
/// content; callers provide parsed feeds, and this type preserves read/starred
/// state across feed refreshes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RssLibrary {
    subscriptions: HashMap<String, RssSubscription>,
    entries: HashMap<RssEntryKey, RssEntryState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RssEntryKey {
    subscription_id: String,
    entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RssError {
    EmptyInput,
    UnsupportedFormat,
    MissingField {
        field: String,
    },
    InvalidSubscription {
        field: String,
    },
    SubscriptionNotFound {
        subscription_id: String,
    },
    EntryNotFound {
        subscription_id: String,
        entry_id: String,
    },
}

impl std::fmt::Display for RssError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RssError::EmptyInput => write!(f, "RSS input is empty"),
            RssError::UnsupportedFormat => write!(f, "unsupported RSS/Atom format"),
            RssError::MissingField { field } => write!(f, "missing RSS field: {field}"),
            RssError::InvalidSubscription { field } => {
                write!(f, "invalid RSS subscription field: {field}")
            }
            RssError::SubscriptionNotFound { subscription_id } => {
                write!(f, "RSS subscription not found: {subscription_id}")
            }
            RssError::EntryNotFound {
                subscription_id,
                entry_id,
            } => {
                write!(
                    f,
                    "RSS entry not found: subscription={subscription_id} entry={entry_id}"
                )
            }
        }
    }
}

impl std::error::Error for RssError {}

impl RssSubscription {
    pub fn new(
        subscription_id: impl Into<String>,
        feed_url: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<Self, RssError> {
        let subscription_id = subscription_id.into().trim().to_string();
        let feed_url = feed_url.into().trim().to_string();
        let title = title.into().trim().to_string();
        validate_subscription_fields(&subscription_id, &feed_url)?;
        Ok(Self {
            subscription_id,
            title: if title.is_empty() {
                feed_url.clone()
            } else {
                title
            },
            feed_url,
            site_url: None,
            enabled: true,
            last_fetch_at: None,
            last_entry_id: None,
            unread_count: 0,
        })
    }

    /// Merge parsed feed metadata and unread state into this subscription.
    ///
    /// Feed entries are assumed to be ordered newest-first. New entries are the
    /// prefix before the previously observed `last_entry_id`; if that id has
    /// fallen out of the feed window, the current feed is treated as all new.
    pub fn apply_feed(
        &mut self,
        feed: &RssFeed,
        fetched_at: i64,
    ) -> Result<RssRefreshResult, RssError> {
        validate_subscription_fields(&self.subscription_id, &self.feed_url)?;
        if feed.title.trim().is_empty() {
            return Err(RssError::MissingField {
                field: "feed.title".into(),
            });
        }

        let new_entries = collect_new_entries(&feed.entries, self.last_entry_id.as_deref());
        self.title = feed.title.clone();
        if let Some(feed_url) = feed.feed_url.as_ref().filter(|url| !url.trim().is_empty()) {
            self.feed_url = feed_url.clone();
        }
        if let Some(site_url) = feed.site_url.as_ref().filter(|url| !url.trim().is_empty()) {
            self.site_url = Some(site_url.clone());
        }
        self.last_fetch_at = Some(fetched_at);
        if let Some(entry) = feed.entries.first() {
            self.last_entry_id = Some(entry.id.clone());
        }
        self.unread_count = self.unread_count.saturating_add(new_entries.len() as u32);

        Ok(RssRefreshResult {
            subscription: self.clone(),
            new_entries,
        })
    }

    pub fn mark_all_read(&mut self) {
        self.unread_count = 0;
    }
}

impl RssLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_subscription(
        &mut self,
        subscription: RssSubscription,
    ) -> Result<RssSubscription, RssError> {
        validate_subscription_fields(&subscription.subscription_id, &subscription.feed_url)?;
        self.subscriptions
            .insert(subscription.subscription_id.clone(), subscription.clone());
        Ok(subscription)
    }

    pub fn get_subscription(
        &self,
        subscription_id: &str,
    ) -> Result<Option<RssSubscription>, RssError> {
        validate_subscription_id(subscription_id)?;
        Ok(self.subscriptions.get(subscription_id).cloned())
    }

    pub fn list_subscriptions(&self) -> Vec<RssSubscription> {
        let mut subscriptions = self.subscriptions.values().cloned().collect::<Vec<_>>();
        subscriptions.sort_by(|a, b| {
            a.title
                .cmp(&b.title)
                .then_with(|| a.subscription_id.cmp(&b.subscription_id))
        });
        subscriptions
    }

    pub fn remove_subscription(&mut self, subscription_id: &str) -> Result<usize, RssError> {
        validate_subscription_id(subscription_id)?;
        self.subscriptions.remove(subscription_id);
        let before = self.entries.len();
        self.entries
            .retain(|key, _| key.subscription_id != subscription_id);
        Ok(before - self.entries.len())
    }

    pub fn refresh_subscription(
        &mut self,
        subscription_id: &str,
        feed: &RssFeed,
        fetched_at: i64,
    ) -> Result<RssRefreshResult, RssError> {
        validate_subscription_id(subscription_id)?;
        let subscription = self
            .subscriptions
            .get(subscription_id)
            .cloned()
            .ok_or_else(|| RssError::SubscriptionNotFound {
                subscription_id: subscription_id.to_string(),
            })?;
        let before_entry_ids = self
            .entries
            .keys()
            .filter(|key| key.subscription_id == subscription_id)
            .map(|key| key.entry_id.clone())
            .collect::<HashSet<_>>();

        let mut updated_subscription = subscription;
        updated_subscription.apply_feed(feed, fetched_at)?;

        let mut actual_new_entries = Vec::new();
        for entry in &feed.entries {
            let key = RssEntryKey {
                subscription_id: subscription_id.to_string(),
                entry_id: entry.id.clone(),
            };
            if let Some(state) = self.entries.get_mut(&key) {
                state.entry = entry.clone();
            } else {
                actual_new_entries.push(entry.clone());
                self.entries.insert(
                    key,
                    RssEntryState {
                        subscription_id: subscription_id.to_string(),
                        entry: entry.clone(),
                        first_seen_at: fetched_at,
                        read: false,
                        read_at: None,
                        starred: false,
                    },
                );
            }
        }

        updated_subscription.unread_count = self.unread_count(subscription_id);
        self.subscriptions
            .insert(subscription_id.to_string(), updated_subscription.clone());

        Ok(RssRefreshResult {
            subscription: updated_subscription,
            new_entries: actual_new_entries
                .into_iter()
                .filter(|entry| !before_entry_ids.contains(&entry.id))
                .collect(),
        })
    }

    pub fn list_entries(&self, subscription_id: &str) -> Result<Vec<RssEntryState>, RssError> {
        validate_subscription_id(subscription_id)?;
        if !self.subscriptions.contains_key(subscription_id) {
            return Err(RssError::SubscriptionNotFound {
                subscription_id: subscription_id.to_string(),
            });
        }
        let mut entries = self
            .entries
            .values()
            .filter(|state| state.subscription_id == subscription_id)
            .cloned()
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| {
            b.first_seen_at
                .cmp(&a.first_seen_at)
                .then_with(|| a.entry.id.cmp(&b.entry.id))
        });
        Ok(entries)
    }

    pub fn mark_entry_read(
        &mut self,
        subscription_id: &str,
        entry_id: &str,
        read_at: i64,
    ) -> Result<RssEntryState, RssError> {
        self.update_entry(subscription_id, entry_id, |state| {
            state.read = true;
            state.read_at = Some(read_at);
        })
    }

    pub fn mark_entry_unread(
        &mut self,
        subscription_id: &str,
        entry_id: &str,
    ) -> Result<RssEntryState, RssError> {
        self.update_entry(subscription_id, entry_id, |state| {
            state.read = false;
            state.read_at = None;
        })
    }

    pub fn mark_all_read(&mut self, subscription_id: &str, read_at: i64) -> Result<(), RssError> {
        validate_subscription_id(subscription_id)?;
        if !self.subscriptions.contains_key(subscription_id) {
            return Err(RssError::SubscriptionNotFound {
                subscription_id: subscription_id.to_string(),
            });
        }
        for state in self.entries.values_mut() {
            if state.subscription_id == subscription_id {
                state.read = true;
                state.read_at = Some(read_at);
            }
        }
        self.recompute_unread_count(subscription_id);
        Ok(())
    }

    pub fn set_entry_starred(
        &mut self,
        subscription_id: &str,
        entry_id: &str,
        starred: bool,
    ) -> Result<RssEntryState, RssError> {
        self.update_entry(subscription_id, entry_id, |state| {
            state.starred = starred;
        })
    }

    fn update_entry(
        &mut self,
        subscription_id: &str,
        entry_id: &str,
        update: impl FnOnce(&mut RssEntryState),
    ) -> Result<RssEntryState, RssError> {
        validate_subscription_id(subscription_id)?;
        validate_entry_id(entry_id)?;
        if !self.subscriptions.contains_key(subscription_id) {
            return Err(RssError::SubscriptionNotFound {
                subscription_id: subscription_id.to_string(),
            });
        }
        let key = RssEntryKey {
            subscription_id: subscription_id.to_string(),
            entry_id: entry_id.to_string(),
        };
        let state = self
            .entries
            .get_mut(&key)
            .ok_or_else(|| RssError::EntryNotFound {
                subscription_id: subscription_id.to_string(),
                entry_id: entry_id.to_string(),
            })?;
        update(state);
        let state = state.clone();
        self.recompute_unread_count(subscription_id);
        Ok(state)
    }

    fn recompute_unread_count(&mut self, subscription_id: &str) {
        let unread_count = self.unread_count(subscription_id);
        if let Some(subscription) = self.subscriptions.get_mut(subscription_id) {
            subscription.unread_count = unread_count;
        }
    }

    fn unread_count(&self, subscription_id: &str) -> u32 {
        self.entries
            .values()
            .filter(|state| state.subscription_id == subscription_id && !state.read)
            .count() as u32
    }
}

/// Parse an RSS 2.0 or Atom feed from an already-fetched XML string.
pub fn parse_feed(xml: &str) -> Result<RssFeed, RssError> {
    parse_feed_inner(xml, None)
}

/// Parse a feed and attach the caller-known feed URL to the result.
pub fn parse_feed_with_url(feed_url: &str, xml: &str) -> Result<RssFeed, RssError> {
    let feed_url = feed_url.trim();
    if feed_url.is_empty() {
        return Err(RssError::InvalidSubscription {
            field: "feed_url".into(),
        });
    }
    parse_feed_inner(xml, Some(feed_url.to_string()))
}

fn parse_feed_inner(xml: &str, provided_feed_url: Option<String>) -> Result<RssFeed, RssError> {
    if xml.trim().is_empty() {
        return Err(RssError::EmptyInput);
    }

    if has_element(xml, "rss") || has_element(xml, "channel") {
        parse_rss_feed(xml, provided_feed_url)
    } else if has_element(xml, "feed") {
        parse_atom_feed(xml, provided_feed_url)
    } else {
        Err(RssError::UnsupportedFormat)
    }
}

fn parse_rss_feed(xml: &str, feed_url: Option<String>) -> Result<RssFeed, RssError> {
    let channel = first_element_body(xml, "channel").unwrap_or_else(|| xml.to_string());
    let channel_metadata = remove_element_blocks(&channel, "item");
    let title = required_text(&channel_metadata, "title", "feed.title")?;
    let site_url = first_text(&channel_metadata, "link");
    let description = first_text(&channel_metadata, "description");

    let mut entries = element_bodies(&channel, "item")
        .into_iter()
        .map(|item| parse_rss_item(&item))
        .collect::<Result<Vec<_>, _>>()?;
    dedupe_entries(&mut entries);

    Ok(RssFeed {
        title,
        feed_url,
        site_url,
        description,
        entries,
    })
}

fn parse_rss_item(item: &str) -> Result<RssEntry, RssError> {
    let title = first_text(item, "title").unwrap_or_default();
    let link = first_text(item, "link");
    let guid = first_text(item, "guid");
    let id = guid
        .or_else(|| link.clone())
        .or_else(|| (!title.is_empty()).then(|| title.clone()))
        .ok_or_else(|| RssError::MissingField {
            field: "entry.id".into(),
        })?;
    Ok(RssEntry {
        id,
        title,
        link,
        summary: first_text(item, "description").or_else(|| first_text(item, "content:encoded")),
        published_at: first_text(item, "pubDate").or_else(|| first_text(item, "dc:date")),
    })
}

fn parse_atom_feed(xml: &str, provided_feed_url: Option<String>) -> Result<RssFeed, RssError> {
    let feed = first_element_body(xml, "feed").unwrap_or_else(|| xml.to_string());
    let feed_metadata = remove_element_blocks(&feed, "entry");
    let title = required_text(&feed_metadata, "title", "feed.title")?;
    let feed_url = provided_feed_url.or_else(|| link_href_by_rel(&feed_metadata, "self"));
    let site_url =
        link_href_by_rel(&feed_metadata, "alternate").or_else(|| first_link_href(&feed_metadata));
    let description = first_text(&feed_metadata, "subtitle");

    let mut entries = element_bodies(&feed, "entry")
        .into_iter()
        .map(|entry| parse_atom_entry(&entry))
        .collect::<Result<Vec<_>, _>>()?;
    dedupe_entries(&mut entries);

    Ok(RssFeed {
        title,
        feed_url,
        site_url,
        description,
        entries,
    })
}

fn parse_atom_entry(entry: &str) -> Result<RssEntry, RssError> {
    let title = first_text(entry, "title").unwrap_or_default();
    let link = link_href_by_rel(entry, "alternate").or_else(|| first_link_href(entry));
    let id = first_text(entry, "id")
        .or_else(|| link.clone())
        .or_else(|| (!title.is_empty()).then(|| title.clone()))
        .ok_or_else(|| RssError::MissingField {
            field: "entry.id".into(),
        })?;
    Ok(RssEntry {
        id,
        title,
        link,
        summary: first_text(entry, "summary").or_else(|| first_text(entry, "content")),
        published_at: first_text(entry, "updated").or_else(|| first_text(entry, "published")),
    })
}

fn required_text(input: &str, tag: &str, field: &str) -> Result<String, RssError> {
    first_text(input, tag).ok_or_else(|| RssError::MissingField {
        field: field.into(),
    })
}

fn collect_new_entries(entries: &[RssEntry], last_entry_id: Option<&str>) -> Vec<RssEntry> {
    let Some(last_entry_id) = last_entry_id else {
        return entries.to_vec();
    };

    let mut new_entries = Vec::new();
    for entry in entries {
        if entry.id == last_entry_id {
            break;
        }
        new_entries.push(entry.clone());
    }
    new_entries
}

fn dedupe_entries(entries: &mut Vec<RssEntry>) {
    let mut seen = HashSet::new();
    entries.retain(|entry| seen.insert(entry.id.clone()));
}

fn validate_subscription_fields(subscription_id: &str, feed_url: &str) -> Result<(), RssError> {
    validate_subscription_id(subscription_id)?;
    if feed_url.trim().is_empty() {
        return Err(RssError::InvalidSubscription {
            field: "feed_url".into(),
        });
    }
    Ok(())
}

fn validate_subscription_id(subscription_id: &str) -> Result<(), RssError> {
    if subscription_id.trim().is_empty() {
        return Err(RssError::InvalidSubscription {
            field: "subscription_id".into(),
        });
    }
    Ok(())
}

fn validate_entry_id(entry_id: &str) -> Result<(), RssError> {
    if entry_id.trim().is_empty() {
        return Err(RssError::InvalidSubscription {
            field: "entry_id".into(),
        });
    }
    Ok(())
}

fn first_text(input: &str, tag: &str) -> Option<String> {
    first_element_body(input, tag).and_then(|body| {
        let text = clean_text(&body);
        (!text.is_empty()).then_some(text)
    })
}

fn first_element_body(input: &str, tag: &str) -> Option<String> {
    element_bodies(input, tag).into_iter().next()
}

fn element_bodies(input: &str, tag: &str) -> Vec<String> {
    let mut bodies = Vec::new();
    let mut from = 0usize;
    while let Some(start) = find_start_tag(input, tag, from) {
        if start.self_closing {
            from = start.open_end + 1;
            continue;
        }
        let Some((close_start, close_end)) = find_end_tag(input, tag, start.content_start) else {
            break;
        };
        bodies.push(input[start.content_start..close_start].to_string());
        from = close_end;
    }
    bodies
}

fn remove_element_blocks(input: &str, tag: &str) -> String {
    let mut output = String::new();
    let mut from = 0usize;
    while let Some(start) = find_start_tag(input, tag, from) {
        output.push_str(&input[from..start.open_start]);
        if start.self_closing {
            from = start.open_end + 1;
            continue;
        }
        let Some((_, close_end)) = find_end_tag(input, tag, start.content_start) else {
            from = start.open_end + 1;
            continue;
        };
        from = close_end;
    }
    output.push_str(&input[from..]);
    output
}

fn has_element(input: &str, tag: &str) -> bool {
    find_start_tag(input, tag, 0).is_some()
}

fn first_link_href(input: &str) -> Option<String> {
    link_start_tags(input)
        .into_iter()
        .find_map(|tag| attr_value(&tag, "href"))
}

fn link_href_by_rel(input: &str, rel: &str) -> Option<String> {
    link_start_tags(input).into_iter().find_map(|tag| {
        let tag_rel = attr_value(&tag, "rel")?;
        tag_rel
            .eq_ignore_ascii_case(rel)
            .then(|| attr_value(&tag, "href"))
            .flatten()
    })
}

fn link_start_tags(input: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut from = 0usize;
    while let Some(start) = find_start_tag(input, "link", from) {
        tags.push(input[start.open_start..=start.open_end].to_string());
        from = start.open_end + 1;
    }
    tags
}

#[derive(Debug, Clone, Copy)]
struct StartTag {
    open_start: usize,
    open_end: usize,
    content_start: usize,
    self_closing: bool,
}

fn find_start_tag(input: &str, tag: &str, from: usize) -> Option<StartTag> {
    let lower_input = input.to_ascii_lowercase();
    let lower_tag = tag.to_ascii_lowercase();
    let needle = format!("<{lower_tag}");
    let mut search_from = from;

    while search_from < input.len() {
        let relative = lower_input[search_from..].find(&needle)?;
        let open_start = search_from + relative;
        let name_end = open_start + needle.len();
        if !is_tag_boundary(input, name_end) {
            search_from = name_end;
            continue;
        }
        let open_end = input[open_start..].find('>')? + open_start;
        let start_tag = &input[open_start..=open_end];
        return Some(StartTag {
            open_start,
            open_end,
            content_start: open_end + 1,
            self_closing: start_tag.trim_end().ends_with("/>"),
        });
    }

    None
}

fn find_end_tag(input: &str, tag: &str, from: usize) -> Option<(usize, usize)> {
    let lower_input = input.to_ascii_lowercase();
    let needle = format!("</{}>", tag.to_ascii_lowercase());
    let relative = lower_input[from..].find(&needle)?;
    let close_start = from + relative;
    Some((close_start, close_start + needle.len()))
}

fn is_tag_boundary(input: &str, index: usize) -> bool {
    input[index..]
        .chars()
        .next()
        .map(|ch| ch == '>' || ch == '/' || ch.is_ascii_whitespace())
        .unwrap_or(false)
}

fn attr_value(start_tag: &str, attr: &str) -> Option<String> {
    let lower = start_tag.to_ascii_lowercase();
    let needle = attr.to_ascii_lowercase();
    let mut from = 0usize;

    while from < start_tag.len() {
        let relative = lower[from..].find(&needle)?;
        let name_start = from + relative;
        let name_end = name_start + needle.len();
        if !is_attr_boundary(start_tag, name_start, name_end) {
            from = name_end;
            continue;
        }

        let mut cursor = name_end;
        cursor = skip_ascii_ws(start_tag, cursor);
        if start_tag[cursor..].chars().next() != Some('=') {
            from = name_end;
            continue;
        }
        cursor += 1;
        cursor = skip_ascii_ws(start_tag, cursor);
        let quote = start_tag[cursor..].chars().next()?;
        if quote != '"' && quote != '\'' {
            from = cursor;
            continue;
        }
        cursor += quote.len_utf8();
        let end_relative = start_tag[cursor..].find(quote)?;
        let raw = &start_tag[cursor..cursor + end_relative];
        return Some(clean_text(raw));
    }

    None
}

fn is_attr_boundary(input: &str, start: usize, end: usize) -> bool {
    let before_ok = input[..start]
        .chars()
        .next_back()
        .map(|ch| ch == '<' || ch.is_ascii_whitespace())
        .unwrap_or(true);
    let after_ok = input[end..]
        .chars()
        .next()
        .map(|ch| ch == '=' || ch.is_ascii_whitespace())
        .unwrap_or(false);
    before_ok && after_ok
}

fn skip_ascii_ws(input: &str, mut cursor: usize) -> usize {
    while cursor < input.len() {
        let Some(ch) = input[cursor..].chars().next() else {
            break;
        };
        if !ch.is_ascii_whitespace() {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

fn clean_text(raw: &str) -> String {
    decode_xml_entities(&strip_cdata(raw.trim()))
        .trim()
        .to_string()
}

fn strip_cdata(input: &str) -> String {
    let mut text = input.trim().to_string();
    if text.starts_with("<![CDATA[") && text.ends_with("]]>") && text.len() >= 12 {
        text = text[9..text.len() - 3].to_string();
    }
    text
}

fn decode_xml_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_channel_and_items() {
        let xml = r#"
            <rss version="2.0">
              <channel>
                <title>Reader &amp; Core</title>
                <link>https://example.test</link>
                <description><![CDATA[Daily updates]]></description>
                <item>
                  <title>Entry 1</title>
                  <link>https://example.test/1</link>
                  <guid isPermaLink="false">entry-1</guid>
                  <pubDate>Wed, 24 Jun 2026 10:00:00 GMT</pubDate>
                  <description><![CDATA[Summary &amp; details]]></description>
                </item>
                <item>
                  <title>Entry 2</title>
                  <link>https://example.test/2</link>
                </item>
              </channel>
            </rss>
        "#;

        let feed = parse_feed_with_url("https://example.test/feed.xml", xml).unwrap();
        assert_eq!(feed.title, "Reader & Core");
        assert_eq!(
            feed.feed_url.as_deref(),
            Some("https://example.test/feed.xml")
        );
        assert_eq!(feed.site_url.as_deref(), Some("https://example.test"));
        assert_eq!(feed.description.as_deref(), Some("Daily updates"));
        assert_eq!(feed.entries.len(), 2);
        assert_eq!(feed.entries[0].id, "entry-1");
        assert_eq!(
            feed.entries[0].summary.as_deref(),
            Some("Summary & details")
        );
        assert_eq!(feed.entries[1].id, "https://example.test/2");
    }

    #[test]
    fn parses_atom_feed_and_link_attributes() {
        let xml = r#"
            <feed xmlns="http://www.w3.org/2005/Atom">
              <title>Atom Feed</title>
              <subtitle>Updates</subtitle>
              <link rel="self" href="https://example.test/atom.xml" />
              <link rel="alternate" href="https://example.test/" />
              <entry>
                <id>tag:example.test,2026:1</id>
                <title>Atom Entry</title>
                <link rel="alternate" href="https://example.test/a" />
                <updated>2026-06-24T10:00:00Z</updated>
                <summary>Atom summary</summary>
              </entry>
            </feed>
        "#;

        let feed = parse_feed(xml).unwrap();
        assert_eq!(feed.title, "Atom Feed");
        assert_eq!(
            feed.feed_url.as_deref(),
            Some("https://example.test/atom.xml")
        );
        assert_eq!(feed.site_url.as_deref(), Some("https://example.test/"));
        assert_eq!(feed.description.as_deref(), Some("Updates"));
        assert_eq!(feed.entries[0].id, "tag:example.test,2026:1");
        assert_eq!(
            feed.entries[0].link.as_deref(),
            Some("https://example.test/a")
        );
        assert_eq!(
            feed.entries[0].published_at.as_deref(),
            Some("2026-06-24T10:00:00Z")
        );
    }

    #[test]
    fn feed_parser_rejects_empty_and_unknown_input() {
        assert_eq!(parse_feed("").unwrap_err(), RssError::EmptyInput);
        assert_eq!(
            parse_feed("<html><body>not a feed</body></html>").unwrap_err(),
            RssError::UnsupportedFormat
        );
    }

    #[test]
    fn feed_parser_requires_feed_title() {
        let err =
            parse_feed("<rss><channel><item><title>A</title></item></channel></rss>").unwrap_err();
        assert_eq!(
            err,
            RssError::MissingField {
                field: "feed.title".into()
            }
        );
    }

    #[test]
    fn feed_parser_requires_entry_identity_when_item_has_no_stable_fields() {
        let err = parse_feed(
            "<rss><channel><title>Feed</title><item><description>x</description></item></channel></rss>",
        )
        .unwrap_err();
        assert_eq!(
            err,
            RssError::MissingField {
                field: "entry.id".into()
            }
        );
    }

    #[test]
    fn feed_parser_deduplicates_entries_by_id() {
        let xml = r#"
            <rss><channel><title>Feed</title>
              <item><title>A</title><guid>same</guid></item>
              <item><title>B</title><guid>same</guid></item>
              <item><title>C</title><guid>other</guid></item>
            </channel></rss>
        "#;

        let feed = parse_feed(xml).unwrap();
        let titles: Vec<&str> = feed
            .entries
            .iter()
            .map(|entry| entry.title.as_str())
            .collect();
        assert_eq!(titles, vec!["A", "C"]);
    }

    #[test]
    fn subscription_new_rejects_empty_required_fields() {
        assert!(matches!(
            RssSubscription::new("", "https://example.test/feed.xml", "Feed"),
            Err(RssError::InvalidSubscription { .. })
        ));
        assert!(matches!(
            RssSubscription::new("sub", "   ", "Feed"),
            Err(RssError::InvalidSubscription { .. })
        ));
    }

    #[test]
    fn subscription_apply_first_feed_marks_all_entries_unread() {
        let feed = RssFeed {
            title: "Feed".into(),
            feed_url: Some("https://example.test/feed.xml".into()),
            site_url: Some("https://example.test".into()),
            description: None,
            entries: vec![
                RssEntry {
                    id: "3".into(),
                    title: "Three".into(),
                    link: None,
                    summary: None,
                    published_at: None,
                },
                RssEntry {
                    id: "2".into(),
                    title: "Two".into(),
                    link: None,
                    summary: None,
                    published_at: None,
                },
            ],
        };
        let mut subscription =
            RssSubscription::new("sub", "https://old.test/feed.xml", "").unwrap();

        let result = subscription.apply_feed(&feed, 1700000000).unwrap();

        assert_eq!(result.new_entries.len(), 2);
        assert_eq!(subscription.feed_url, "https://example.test/feed.xml");
        assert_eq!(
            subscription.site_url.as_deref(),
            Some("https://example.test")
        );
        assert_eq!(subscription.last_entry_id.as_deref(), Some("3"));
        assert_eq!(subscription.last_fetch_at, Some(1700000000));
        assert_eq!(subscription.unread_count, 2);
    }

    #[test]
    fn subscription_apply_next_feed_counts_only_new_prefix() {
        let mut subscription =
            RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap();
        subscription.last_entry_id = Some("2".into());
        subscription.unread_count = 4;
        let feed = RssFeed {
            title: "Feed".into(),
            feed_url: None,
            site_url: None,
            description: None,
            entries: vec![
                RssEntry {
                    id: "4".into(),
                    title: "Four".into(),
                    link: None,
                    summary: None,
                    published_at: None,
                },
                RssEntry {
                    id: "3".into(),
                    title: "Three".into(),
                    link: None,
                    summary: None,
                    published_at: None,
                },
                RssEntry {
                    id: "2".into(),
                    title: "Two".into(),
                    link: None,
                    summary: None,
                    published_at: None,
                },
            ],
        };

        let result = subscription.apply_feed(&feed, 1700001000).unwrap();

        let ids: Vec<&str> = result
            .new_entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(ids, vec!["4", "3"]);
        assert_eq!(subscription.last_entry_id.as_deref(), Some("4"));
        assert_eq!(subscription.unread_count, 6);
    }

    #[test]
    fn subscription_apply_feed_treats_missing_previous_id_as_all_new() {
        let mut subscription =
            RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap();
        subscription.last_entry_id = Some("old".into());
        let feed = RssFeed {
            title: "Feed".into(),
            feed_url: None,
            site_url: None,
            description: None,
            entries: vec![RssEntry {
                id: "new".into(),
                title: "New".into(),
                link: None,
                summary: None,
                published_at: None,
            }],
        };

        let result = subscription.apply_feed(&feed, 1700002000).unwrap();

        assert_eq!(result.new_entries.len(), 1);
        assert_eq!(subscription.last_entry_id.as_deref(), Some("new"));
        assert_eq!(subscription.unread_count, 1);
    }

    #[test]
    fn subscription_mark_all_read_resets_unread_count() {
        let mut subscription =
            RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap();
        subscription.unread_count = 12;

        subscription.mark_all_read();

        assert_eq!(subscription.unread_count, 0);
    }

    fn entry(id: &str, title: &str) -> RssEntry {
        RssEntry {
            id: id.into(),
            title: title.into(),
            link: Some(format!("https://example.test/{id}")),
            summary: None,
            published_at: None,
        }
    }

    fn feed(ids: &[&str]) -> RssFeed {
        RssFeed {
            title: "Feed".into(),
            feed_url: Some("https://example.test/feed.xml".into()),
            site_url: Some("https://example.test".into()),
            description: None,
            entries: ids
                .iter()
                .map(|id| entry(id, &format!("Entry {id}")))
                .collect(),
        }
    }

    #[test]
    fn rss_library_upserts_and_lists_subscriptions_deterministically() {
        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("b", "https://example.test/b.xml", "Beta").unwrap(),
            )
            .unwrap();
        library
            .upsert_subscription(
                RssSubscription::new("a", "https://example.test/a.xml", "Alpha").unwrap(),
            )
            .unwrap();

        let ids: Vec<String> = library
            .list_subscriptions()
            .into_iter()
            .map(|subscription| subscription.subscription_id)
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(
            library.get_subscription("a").unwrap().unwrap().title,
            "Alpha"
        );
    }

    #[test]
    fn rss_library_refresh_inserts_entries_and_updates_unread_count() {
        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("sub", "https://old.test/feed.xml", "Old").unwrap(),
            )
            .unwrap();

        let result = library
            .refresh_subscription("sub", &feed(&["3", "2", "1"]), 1000)
            .unwrap();

        assert_eq!(result.new_entries.len(), 3);
        assert_eq!(result.subscription.title, "Feed");
        assert_eq!(
            result.subscription.feed_url,
            "https://example.test/feed.xml"
        );
        assert_eq!(result.subscription.unread_count, 3);
        let states = library.list_entries("sub").unwrap();
        assert_eq!(states.len(), 3);
        assert!(states.iter().all(|state| !state.read));
        assert!(states.iter().all(|state| state.first_seen_at == 1000));
    }

    #[test]
    fn rss_library_refresh_preserves_read_and_starred_state() {
        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap(),
            )
            .unwrap();
        library
            .refresh_subscription("sub", &feed(&["2", "1"]), 1000)
            .unwrap();
        library.mark_entry_read("sub", "1", 1100).unwrap();
        library.set_entry_starred("sub", "1", true).unwrap();

        let result = library
            .refresh_subscription("sub", &feed(&["3", "2", "1"]), 2000)
            .unwrap();

        assert_eq!(
            result
                .new_entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["3"]
        );
        assert_eq!(result.subscription.unread_count, 2);
        let states = library.list_entries("sub").unwrap();
        let one = states.iter().find(|state| state.entry.id == "1").unwrap();
        assert!(one.read);
        assert_eq!(one.read_at, Some(1100));
        assert!(one.starred);
        assert_eq!(one.first_seen_at, 1000);
        let three = states.iter().find(|state| state.entry.id == "3").unwrap();
        assert!(!three.read);
        assert_eq!(three.first_seen_at, 2000);
    }

    #[test]
    fn rss_library_mark_unread_and_all_read_recompute_subscription_count() {
        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap(),
            )
            .unwrap();
        library
            .refresh_subscription("sub", &feed(&["2", "1"]), 1000)
            .unwrap();

        library.mark_entry_read("sub", "1", 1100).unwrap();
        assert_eq!(
            library
                .get_subscription("sub")
                .unwrap()
                .unwrap()
                .unread_count,
            1
        );
        library.mark_entry_unread("sub", "1").unwrap();
        assert_eq!(
            library
                .get_subscription("sub")
                .unwrap()
                .unwrap()
                .unread_count,
            2
        );
        library.mark_all_read("sub", 1200).unwrap();
        assert_eq!(
            library
                .get_subscription("sub")
                .unwrap()
                .unwrap()
                .unread_count,
            0
        );
        assert!(library
            .list_entries("sub")
            .unwrap()
            .iter()
            .all(|state| state.read_at == Some(1200)));
    }

    #[test]
    fn rss_library_remove_subscription_is_idempotent_and_clears_entries() {
        let mut library = RssLibrary::new();
        library
            .upsert_subscription(
                RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap(),
            )
            .unwrap();
        library
            .refresh_subscription("sub", &feed(&["2", "1"]), 1000)
            .unwrap();

        assert_eq!(library.remove_subscription("sub").unwrap(), 2);
        assert!(library.get_subscription("sub").unwrap().is_none());
        assert_eq!(library.remove_subscription("sub").unwrap(), 0);
    }

    #[test]
    fn rss_library_reports_missing_subscription_and_entry() {
        let mut library = RssLibrary::new();
        assert_eq!(
            library
                .refresh_subscription("missing", &feed(&["1"]), 1000)
                .unwrap_err(),
            RssError::SubscriptionNotFound {
                subscription_id: "missing".into()
            }
        );
        assert_eq!(
            library.list_entries("missing").unwrap_err(),
            RssError::SubscriptionNotFound {
                subscription_id: "missing".into()
            }
        );

        library
            .upsert_subscription(
                RssSubscription::new("sub", "https://example.test/feed.xml", "Feed").unwrap(),
            )
            .unwrap();
        assert_eq!(
            library.mark_entry_read("sub", "missing", 1).unwrap_err(),
            RssError::EntryNotFound {
                subscription_id: "sub".into(),
                entry_id: "missing".into()
            }
        );
        assert!(matches!(
            library.get_subscription(""),
            Err(RssError::InvalidSubscription { .. })
        ));
        assert!(matches!(
            library.mark_entry_read("sub", "", 1),
            Err(RssError::InvalidSubscription { .. })
        ));
    }
}
