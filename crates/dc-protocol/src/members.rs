//! Lazy member-list payloads. Unknown rows preserve their positions.
use crate::{Snowflake, model::User};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Member {
    pub user: User,
    #[serde(default)]
    pub nick: Option<String>,
    #[serde(default)]
    pub roles: Vec<Snowflake>,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Role {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: u32,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub hoist: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CustomEmoji {
    pub id: Snowflake,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub animated: bool,
    #[serde(default = "yes")]
    pub available: bool,
    #[serde(default)]
    pub roles: Vec<Snowflake>,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct MemberGroup {
    pub id: String,
    #[serde(default)]
    pub count: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Presence {
    #[serde(default)]
    pub status: String,
}
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct MemberItem {
    #[serde(default)]
    pub member: Option<Member>,
    #[serde(default)]
    pub group: Option<MemberGroup>,
    #[serde(default)]
    pub presence: Option<Presence>,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct MemberUpdate {
    pub guild_id: Snowflake,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub member_count: u64,
    #[serde(default)]
    pub ops: Vec<MemberOp>,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "op")]
pub enum MemberOp {
    #[serde(rename = "SYNC")]
    Sync {
        range: [usize; 2],
        items: Vec<MemberItem>,
    },
    #[serde(rename = "INSERT")]
    Insert { index: usize, item: MemberItem },
    #[serde(rename = "UPDATE")]
    Update { index: usize, item: MemberItem },
    #[serde(rename = "DELETE")]
    Delete { index: usize },
    #[serde(rename = "INVALIDATE")]
    Invalidate { range: [usize; 2] },
    #[serde(other)]
    Unknown,
}
