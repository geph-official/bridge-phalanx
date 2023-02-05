use std::{collections::HashMap, ops::Deref, time::Duration};

use crate::{
    database::{BridgeInfo, DATABASE},
    ssh::ssh_execute,
};

pub async fn loop_onoff() {
    let mut last_status: HashMap<String, String> = HashMap::new();
    loop {
        if let Err(err) = loop_onoff_once(&mut last_status).await {
            log::warn!("error: {:?}", err)
        }
    }
}

/// Synchronizes the in-database status of bridges with whether their systemd service is on.
async fn loop_onoff_once(last_status: &mut HashMap<String, String>) -> anyhow::Result<()> {
    loop {
        let all_bridges: Vec<BridgeInfo> = sqlx::query_as("select * from bridges")
            .fetch_all(DATABASE.deref())
            .await?;
        let mut tasks = vec![];
        for bridge in all_bridges {
            let old_status = last_status.insert(bridge.bridge_id.clone(), bridge.status.clone());
            if old_status != Some(bridge.status.clone()) {
                let task = smol::spawn(async move {
                    log::debug!(
                        "{} ({}) transitions {} => {}",
                        bridge.bridge_id,
                        bridge.ip_addr,
                        old_status.unwrap_or_else(|| "(none)".into()),
                        bridge.status
                    );
                    match bridge.status.as_str() {
                        "frontline" => {
                            ssh_execute(
                            &bridge.ip_addr,
                            "systemctl enable geph4-bridge; systemctl is-active --quiet geph4-bridge || systemctl start geph4-bridge",
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
                    anyhow::Ok(())
                });
                tasks.push(task);
            }
        }

        for task in tasks {
            task.await?;
        }

        smol::Timer::after(Duration::from_secs_f64(fastrand::f64() * 1.0 + 1.0)).await;
    }
}
