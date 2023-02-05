use chrono::NaiveDateTime;
use once_cell::sync::Lazy;
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};

use crate::config::CONFIG;

/// The global database instance.
pub static DATABASE: Lazy<Pool<Postgres>> = Lazy::new(|| {
    PgPoolOptions::new()
        .max_connections(2)
        .connect_lazy(&CONFIG.postgres_url)
        .unwrap()
});

/// Info about a particular bridge, stored in the database.
#[derive(sqlx::FromRow)]
pub struct BridgeInfo {
    pub bridge_id: String,
    pub ip_addr: String,
    pub alloc_group: String,
    pub status: String,
    pub change_time: NaiveDateTime,
}
