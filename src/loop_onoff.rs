use std::{ops::Deref, time::Duration};

use anyhow::Context;
use dashmap::DashMap;
use futures_util::{StreamExt, TryFutureExt, TryStreamExt};
use rand::seq::SliceRandom;
use smol_timeout::TimeoutExt;

use crate::{
    database::{BridgeInfo, DATABASE},
    ssh::ssh_execute,
};

pub async fn loop_onoff() {
    let last_status: DashMap<String, String> = DashMap::new();
    loop {
        if let Err(err) = async {
            anyhow::Ok(
                loop_onoff_once(&last_status)
                    .timeout(Duration::from_secs(120))
                    .await
                    .context("timeout")??,
            )
        }
        .await
        {
            log::warn!("error: {:?}", err)
        }
        smol::Timer::after(Duration::from_secs_f64(fastrand::f64() * 1.0 + 1.0)).await;
    }
}

/// Synchronizes the in-database status of bridges with whether their systemd service is on.
async fn loop_onoff_once(last_status: &DashMap<String, String>) -> anyhow::Result<()> {
    let mut all_bridges: Vec<BridgeInfo> = sqlx::query_as("select * from bridges")
        .fetch_all(DATABASE.deref())
        .await?;

    all_bridges.shuffle(&mut rand::thread_rng());

    let mut tasks = vec![];
    let total = all_bridges.len();
    for (i, bridge) in all_bridges.into_iter().enumerate() {
        let old_status = last_status
            .get(&bridge.bridge_id.clone())
            .map(|s| s.clone());
        if old_status != Some(bridge.status.clone()) {
            let bb = bridge.clone();
            let task = async move {
                log::debug!(
                    "{i}/{total} {}/{} ({}) transitions {} => {}",
                    bridge.alloc_group,
                    bridge.bridge_id,
                    bridge.ip_addr,
                    old_status.unwrap_or_else(|| "(none)".into()),
                    bridge.status
                );
                match bridge.status.as_str() {
                    "frontline" => {
                        ssh_execute(
                            &bridge.ip_addr,
                            "systemctl enable geph4-bridge; (systemctl is-active --quiet geph4-bridge || systemctl start geph4-bridge)",
                        )
                        .await?;
                    }
                    "blocked" | "reserve" => {
                        ssh_execute(
                            &bridge.ip_addr,
                            "systemctl stop geph4-bridge; systemctl disable geph4-bridge",
                        )
                        .await?;
                    }
                    other => {
                        log::debug!("noop for other status {other}")
                    }
                }
                last_status.insert(bridge.bridge_id, bridge.status);
                anyhow::Ok(())
            };
            tasks.push(task.map_err(move |e| {
                log::warn!("{}/{} failed: {e}", bb.alloc_group, bb.ip_addr);
                e
            }));
        }
    }
    let v: Vec<()> = futures_util::stream::iter(tasks)
        .buffered(64)
        .try_collect()
        .await?;
    log::debug!("{} statuses updated", v.len());
    Ok(())
}
