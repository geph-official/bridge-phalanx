use std::{ops::Deref, time::Duration};

use futures_util::future::join_all;

use crate::{
    config::{GroupConfig, CONFIG},
    database::DATABASE,
};

pub async fn loop_prune() {
    // let _all = smol::spawn(loop_prune_all());
    join_all(
        CONFIG
            .groups
            .iter()
            .map(|(gname, gcfg)| loop_prune_for_group(gname, gcfg)),
    )
    .await;
}

// async fn loop_prune_all() {
//     loop {
//         if let Err(err) = sqlx::query("delete from bridges where status = 'blocked'")
//             .execute(DATABASE.deref())
//             .await
//         {
//             log::warn!("prune_all error: {:?}", err);
//         }
//         smol::Timer::after(Duration::from_secs(1)).await;
//     }
// }

async fn loop_prune_for_group(group_name: &str, group_config: &GroupConfig) {
    let (total_group_count,): (i64,) =
        sqlx::query_as("select count(*) from bridges where alloc_group = $1")
            .bind(group_name)
            .fetch_one(DATABASE.deref())
            .await
            .unwrap();
    let delete_interval = group_config.avg_lifetime_hr / (total_group_count.max(1) as f64) * 3600.0;
    let mut timer = smol::Timer::interval(Duration::from_secs_f64(delete_interval));
    loop {
        (&mut timer).await;
        log::debug!("prune timer fires for {group_name} with delete_interval {delete_interval}");
        if let Err(err) = sqlx::query(
            "delete from bridges where alloc_group = $1 and change_time = (
        SELECT MIN(last_mbps)
        FROM bridges
        WHERE alloc_group = $2
        AND last_mbps > 1
      ) limit 1",
        )
        .bind(group_name)
        .bind(group_name)
        .execute(DATABASE.deref())
        .await
        {
            log::warn!("prune error for {group_name}: {:?}", err);
        }
    }
}
