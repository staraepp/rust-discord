//! Bounded reducer for lazy member-list slots, including group headers.
use dc_protocol::members::{MemberItem, MemberOp, MemberUpdate};
pub const MAX_MEMBER_SLOTS: usize = 2000;
#[derive(Clone, Debug, Default)]
pub struct MemberList {
    pub id: String,
    pub total: u64,
    pub slots: Vec<Option<MemberItem>>,
}
impl MemberList {
    pub fn apply(&mut self, update: &MemberUpdate) {
        if self.id != update.id {
            self.slots.clear();
            self.id.clone_from(&update.id);
        }
        self.total = update.member_count;
        for op in &update.ops {
            match op {
                MemberOp::Sync {
                    range: [start, end],
                    items,
                } if start <= end && *end < MAX_MEMBER_SLOTS => {
                    self.slots.resize(self.slots.len().max(end + 1), None);
                    self.slots[*start..=*end].fill(None);
                    for (slot, item) in self.slots[*start..=*end].iter_mut().zip(items) {
                        *slot = Some(item.clone());
                    }
                }
                MemberOp::Update { index, item } if *index < MAX_MEMBER_SLOTS => {
                    self.slots.resize(self.slots.len().max(index + 1), None);
                    self.slots[*index] = Some(item.clone());
                }
                MemberOp::Insert { index, item }
                    if *index <= self.slots.len() && *index < MAX_MEMBER_SLOTS =>
                {
                    self.slots.insert(*index, Some(item.clone()));
                    self.slots.truncate(MAX_MEMBER_SLOTS);
                }
                MemberOp::Delete { index } if *index < self.slots.len() => {
                    self.slots.remove(*index);
                }
                MemberOp::Invalidate {
                    range: [start, end],
                } if start <= end => {
                    let end = (*end).min(self.slots.len().saturating_sub(1));
                    if *start <= end && !self.slots.is_empty() {
                        self.slots[*start..=end].fill(None);
                    }
                }
                _ => {}
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_insert_delete_invalidate_preserve_slots_and_bound_memory() {
        let mut list = MemberList::default();
        let update: MemberUpdate = serde_json::from_value(serde_json::json!({"guild_id":"1","id":"everyone","member_count":2,"ops":[{"op":"SYNC","range":[0,3],"items":[{"group":{"id":"online","count":2}},{"member":{"user":{"id":"4","username":"Test"}}},{}]}]})).unwrap();
        list.apply(&update);
        assert_eq!(list.slots.len(), 4);
        assert!(list.slots[3].is_none());
        let mut update = update;
        update.ops = vec![
            MemberOp::Insert {
                index: 1,
                item: MemberItem::default(),
            },
            MemberOp::Delete { index: 0 },
            MemberOp::Invalidate { range: [1, 2] },
            MemberOp::Update {
                index: usize::MAX,
                item: MemberItem::default(),
            },
        ];
        list.apply(&update);
        assert_eq!(list.slots.len(), 4);
        assert!(list.slots[1].is_none());
        assert!(list.slots[2].is_none());
    }
}
