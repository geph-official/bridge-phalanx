use std::{ops::Deref, time::Duration};

use crate::{
    config::GroupConfig,
    database::{BridgeInfo, DATABASE},
};

pub async fn loop_frontline(alloc_group: String, cfg: GroupConfig) {
    loop {
        if let Err(err) = loop_frontline_inner(&alloc_group, &cfg).await {
            log::warn!("error: {:?}", err)
        }
        smol::Timer::after(Duration::from_secs(1)).await;
    }
}

async fn loop_frontline_inner(alloc_group: &str, cfg: &GroupConfig) -> anyhow::Result<()> {
    // when not enough is in the frontline, move to frontline
    let (frontline_count,): (i64,) = sqlx::query_as(
        "select count(bridge_id) from bridges where status = 'frontline' and alloc_group = $1",
    )
    .bind(alloc_group)
    .fetch_one(DATABASE.deref())
    .await?;
    if frontline_count < cfg.frontline as i64 {
        // attempting to move to frontline
        let movable: Option<BridgeInfo> = sqlx::query_as(
            "select * from bridges where status = 'reserve' and alloc_group = $1 limit 1",
        )
        .bind(alloc_group)
        .fetch_optional(DATABASE.deref())
        .await?;
        if let Some(movable) = movable {
            sqlx::query(
                "update bridges set status = 'frontline', change_time = NOW() where bridge_id = $1",
            )
            .bind(movable.bridge_id)
            .execute(DATABASE.deref())
            .await?;
        }
    }
    Ok(())
}
