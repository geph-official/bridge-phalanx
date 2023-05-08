use std::{ops::Deref, time::Duration};

use crate::{config::CONFIG, database::DATABASE};

pub async fn loop_prune() {
    let total_count: usize = CONFIG
        .groups
        .values()
        .map(|v| v.frontline + v.reserve)
        .sum();
    let delete_interval = CONFIG.max_lifetime_hr / (total_count as f64) * 3600.0;
    let mut timer = smol::Timer::interval(Duration::from_secs_f64(delete_interval));
    loop {
        log::debug!("prune timer fires {delete_interval}");
        if let Err(err) = sqlx::query(
            "delete from bridges where change_time = (
            SELECT MIN(change_time)
            FROM bridges
          ) or (status = 'blocked')",
        )
        .execute(DATABASE.deref())
        .await
        {
            log::warn!("prune error: {:?}", err);
        }
        (&mut timer).await;
    }
}
