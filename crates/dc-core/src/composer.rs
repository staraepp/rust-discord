//! Per-channel drafts and explicit human submissions. No IO or automatic sending.
use dc_protocol::{
    Snowflake,
    model::{AllowedMentions, CreateMessage, MessageReference},
};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub content: String,
    pub reply: Option<Snowflake>,
    pub error: Option<String>,
    pub pending: bool,
    nonce: Option<String>,
}

#[derive(Default)]
pub struct Composer {
    drafts: HashMap<Snowflake, Draft>,
    sequence: u64,
}

impl Composer {
    pub fn draft(&self, channel: Snowflake) -> Draft {
        self.drafts.get(&channel).cloned().unwrap_or_default()
    }
    pub fn edit(&mut self, channel: Snowflake, content: String) {
        let draft = self.drafts.entry(channel).or_default();
        if draft.pending {
            return;
        }
        if draft.content != content {
            draft.nonce = None;
        }
        draft.content = content;
        draft.error = None;
    }
    pub fn reply(&mut self, channel: Snowflake, message: Option<Snowflake>) {
        let draft = self.drafts.entry(channel).or_default();
        if !draft.pending {
            draft.reply = message;
            draft.nonce = None;
        }
    }
    pub fn submit(
        &mut self,
        channel: Snowflake,
        unix_ms: u64,
    ) -> Result<CreateMessage, &'static str> {
        self.submit_with_files(channel, unix_ms, false)
    }
    pub fn submit_with_files(
        &mut self,
        channel: Snowflake,
        unix_ms: u64,
        has_files: bool,
    ) -> Result<CreateMessage, &'static str> {
        let draft = self.drafts.entry(channel).or_default();
        if draft.pending {
            return Err("Message is already sending");
        }
        if draft.content.trim().is_empty() && !has_files {
            return Err("Write a message first");
        }
        if draft.content.encode_utf16().count() > 2000 {
            return Err("Message exceeds 2000 characters");
        }
        self.sequence = self.sequence.wrapping_add(1);
        let nonce = draft
            .nonce
            .get_or_insert_with(|| format!("{unix_ms}{}", self.sequence % 1_000_000));
        draft.pending = true;
        draft.error = None;
        Ok(CreateMessage {
            content: draft.content.clone(),
            nonce: nonce.clone(),
            message_reference: draft.reply.map(|id| MessageReference {
                message_id: id,
                channel_id: channel,
                fail_if_not_exists: false,
            }),
            allowed_mentions: AllowedMentions::default(),
        })
    }
    pub fn complete(&mut self, channel: Snowflake, nonce: &str, error: Option<String>) -> bool {
        let Some(draft) = self.drafts.get_mut(&channel) else {
            return false;
        };
        if draft.nonce.as_deref() != Some(nonce) {
            return false;
        }
        draft.pending = false;
        match error {
            Some(error) => draft.error = Some(error),
            None => *draft = Draft::default(),
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failures_preserve_content_and_nonce_for_manual_retry() {
        let mut c = Composer::default();
        let channel = Snowflake(1);
        c.edit(channel, "hello 😀".into());
        c.reply(channel, Some(Snowflake(4)));
        let first = c.submit(channel, 100).unwrap();
        assert!(c.submit(channel, 101).is_err());
        assert!(!c.complete(channel, "stale", None));
        assert!(c.complete(channel, &first.nonce, Some("offline".into())));
        let retry = c.submit(channel, 102).unwrap();
        assert_eq!(retry, first);
        assert!(c.complete(channel, &retry.nonce, None));
        assert!(c.draft(channel).content.is_empty());
        assert_eq!(c.draft(channel).reply, None);
    }
    #[test]
    fn drafts_are_independent_and_edits_invalidate_failed_nonces() {
        let mut c = Composer::default();
        let a = Snowflake(1);
        let b = Snowflake(2);
        c.edit(a, "first".into());
        c.edit(b, "second".into());
        let first = c.submit(a, 100).unwrap();
        c.complete(a, &first.nonce, Some("offline".into()));
        c.edit(a, "changed".into());
        let next = c.submit(a, 100).unwrap();
        assert_ne!(next.nonce, first.nonce);
        assert_eq!(c.draft(b).content, "second");
    }
    #[test]
    fn empty_and_overlong_messages_never_become_pending() {
        let mut c = Composer::default();
        let a = Snowflake(1);
        c.edit(a, " \n".into());
        assert!(c.submit(a, 0).is_err());
        c.edit(a, "😀".repeat(1001));
        assert!(c.submit(a, 0).is_err());
        assert!(!c.draft(a).pending);
    }
}
