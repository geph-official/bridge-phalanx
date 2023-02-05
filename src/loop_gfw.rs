use std::{ops::Deref, time::Duration};

use rand::seq::SliceRandom;
use smol::lock::Semaphore;

use crate::{
    database::{BridgeInfo, DATABASE},
    ssh::ssh_execute,
};

pub async fn loop_gfw() {
    loop {
        if let Err(err) = loop_gfw_inner().await {
            log::warn!("error: {:?}", err)
        }
        smol::Timer::after(Duration::from_secs(1)).await;
    }
}

async fn loop_gfw_inner() -> anyhow::Result<()> {
    // test all the bridges in a random order
    let mut bridges: Vec<BridgeInfo> =
        sqlx::query_as("select * from bridges where status = 'reserve' or status = 'frontline' or status = 'blocked'")
            .fetch_all(DATABASE.deref())
            .await?;
    bridges.shuffle(&mut rand::thread_rng());
    let mut tasks = vec![];
    for bridge in bridges {
        static SMALL_SEMAPHORE: Semaphore = Semaphore::new(8);
        tasks.push(smol::spawn(async move {
            let _guard = SMALL_SEMAPHORE.acquire().await;
            let is_blocked = ssh_execute(&bridge.ip_addr, "ping -W 2 -c 10 10010.com || true")
                .await?
                .contains("100%");
            log::debug!(
                "[{}] {} blocked? {is_blocked}",
                bridge.status,
                bridge.ip_addr
            );
            if is_blocked && bridge.status != "blocked" {
                sqlx::query("update bridges set status = 'blocked' where bridge_id = $1")
                    .bind(bridge.bridge_id)
                    .execute(DATABASE.deref())
                    .await?;
            } else if !is_blocked && bridge.status == "blocked" {
                sqlx::query("update bridges set status = 'reserve' where bridge_id = $1")
                    .bind(bridge.bridge_id)
                    .execute(DATABASE.deref())
                    .await?;
            }
            anyhow::Ok(())
        }));
    }
    for task in tasks {
        task.await?;
    }
    Ok(())
}
