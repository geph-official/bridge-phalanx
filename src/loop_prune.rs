use std::{ops::Deref, time::Duration};

use futures_util::future::join_all;

use crate::{
    config::{GroupConfig, CONFIG},
    database::DATABASE,
};

pub async fn loop_prune() {
    join_all(
        CONFIG
            .groups
            .iter()
            .map(|(gname, gcfg)| loop_prune_for_group(gname, gcfg)),
    )
    .await;
}

async fn loop_prune_for_group(group_name: &str, group_config: &GroupConfig) {
    let total_group_count: usize = group_config.frontline + group_config.reserve;
    let delete_interval = group_config.max_lifetime_hr / (total_group_count as f64) * 3600.0;
    let mut timer = smol::Timer::interval(Duration::from_secs_f64(delete_interval));
    loop {
        log::debug!("prune timer fires for {group_name} with delete_interval {delete_interval}");
        if let Err(err) = sqlx::query(
            "delete from bridges where alloc_group = $1 and change_time = (
        SELECT MIN(change_time)
        FROM bridges
        WHERE alloc_group = $2
      ) or (status = 'blocked')",
        )
        .bind(group_name)
        .bind(group_name)
        .execute(DATABASE.deref())
        .await
        {
            log::warn!("prune error for {group_name}: {:?}", err);
        }
        (&mut timer).await;
    }
}
