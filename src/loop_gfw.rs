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
        sqlx::query_as("select * from bridges where status = 'reserve' or status = 'frontline'")
            .fetch_all(DATABASE.deref())
            .await?;
    bridges.shuffle(&mut rand::thread_rng());
    let mut tasks = vec![];
    for bridge in bridges {
        static SMALL_SEMAPHORE: Semaphore = Semaphore::new(32);
        tasks.push(smol::spawn(async move {
            let _guard = SMALL_SEMAPHORE.acquire().await;
            let is_blocked = {
                let blocked_in_china =
                    ssh_execute(&bridge.ip_addr, "ping -W 2 -c 10 10010.com || true")
                        .await?
                        .contains("100%");
                blocked_in_china
                // if !blocked_in_china {
                //     if bridge.status == "frontline" && (bridge.alloc_group.contains("scw")||bridge.alloc_group.contains("eu_north")) ||bridge.alloc_group.contains("hetzner"){
                //     // not blocked in china, but maybe in iran
                //     ssh_execute(&bridge.ip_addr, "apt install -y geoip-bin").await?;
                //     let ir_count: usize = ssh_execute(&bridge.ip_addr, r#"SUBNET=$(ip -o -f inet addr show | grep -v '127.0.0.1' | awk '{gsub(/\/.*/,""); print $4}')
                //     tshark -a duration:2 -i any -T fields -e ip.src -E separator=, -Y "ip.dst==$SUBNET" | while read -r ip; do geoiplookup "$ip"; done | grep -o "Iran" | wc -l
                //     "#).await?.trim().parse()?;
                //     let cn_count: usize = ssh_execute(&bridge.ip_addr, r#"SUBNET=$(ip -o -f inet addr show | grep -v '127.0.0.1' | awk '{gsub(/\/.*/,""); print $4}')
                //     tshark -a duration:2 -i any -T fields -e ip.src -E separator=, -Y "ip.dst==$SUBNET" | while read -r ip; do geoiplookup "$ip"; done | grep -o "China" | wc -l
                //     "#).await?.trim().parse()?;

                //     let heuristic = (ir_count + cn_count) > 200 && ir_count < cn_count/2 ;
                //     log::debug!("{}/{} has ir={ir_count} cn={cn_count}; HEURISTIC = {heuristic}", bridge.alloc_group, bridge.ip_addr);
                //     heuristic
                //     } else {
                //         false
                //     }
                // } else {
                //     true
                // }
            };
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
