use std::{ops::Deref, time::Duration};

use rand::Rng;

use crate::{config::CONFIG, database::DATABASE};

pub async fn loop_prune() {
    loop {
        if let Err(err) = sqlx::query("delete from bridges where status != 'reserve' and change_time + interval '1 hour' * $1  < NOW()").bind(CONFIG.max_lifetime_hr).execute(DATABASE.deref()).await {
            log::warn!("prune error: {:?}", err);
        }
        let count = rand::thread_rng().gen_range(10..20);
        smol::Timer::after(Duration::from_secs(count)).await;
    }
}
