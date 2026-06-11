use serde::Deserialize;

pub mod bulk;
pub mod create;
pub mod detail;
pub mod discord;
pub mod dues;
pub mod list;
pub mod payments;
pub mod status;
pub mod verification;

pub use bulk::*;

#[derive(Clone)]
pub struct MembershipTypeOption {
    pub id: String,
    pub slug: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct AdminMembersQuery {
    pub q: Option<String>,
    pub status: Option<String>,
    #[serde(rename = "type")]
    pub member_type: Option<String>,
    pub page: Option<i64>,
    pub sort: Option<String>,
    pub order: Option<String>,
}
