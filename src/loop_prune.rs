use std::{ops::Deref, time::Duration};

use rand::Rng;

use crate::{config::CONFIG, database::DATABASE};

pub async fn loop_prune() {
    loop {
        for (group_name, group_config) in CONFIG.groups.iter() {
            let total_group_count: usize = group_config.frontline + group_config.reserve;
            let delete_interval =
                group_config.max_lifetime_hr / (total_group_count as f64) * 3600.0;
            let mut timer = smol::Timer::interval(Duration::from_secs_f64(delete_interval));

            log::debug!(
                "prune timer fires for {group_name} with delete_interval {delete_interval}"
            );
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
                log::warn!("prune error: {:?}", err);
            }
            (&mut timer).await;
        }
        let count = rand::thread_rng().gen_range(10..20);
        smol::Timer::after(Duration::from_secs(count)).await;
    }
}
