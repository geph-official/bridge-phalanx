use std::{
    ops::Deref,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::{
    config::GroupConfig,
    database::{BridgeInfo, DATABASE},
    provider::Provider,
    ssh::ssh_execute,
};

pub async fn loop_frontline(alloc_group: String, cfg: GroupConfig, provider: Arc<dyn Provider>) {
    let adjusted_frontline = {
        let (current_live,): (i64,) = sqlx::query_as(
            "select count(*) from bridges where alloc_group = $1 and status = 'frontline'",
        )
        .bind(&alloc_group)
        .fetch_one(DATABASE.deref())
        .await
        .expect("could not fetch current live");
        Arc::new(AtomicUsize::new(cfg.frontline.max(current_live as usize)))
    };
    let _lala_loop = {
        let adjusted_frontline = adjusted_frontline.clone();
        let base_frontline = cfg.frontline;
        if base_frontline == 0 || cfg.target_mbps == 0.0f64 {
            return;
        }

        let alloc_group = alloc_group.clone();
        smol::spawn(async move {
            let mut timer = smol::Timer::interval(Duration::from_secs(600));
            loop {
                let (current_live,): (i64,) = sqlx::query_as(
                    "select count(*) from bridges where alloc_group = $1 and status = 'frontline'",
                )
                .bind(&alloc_group)
                .fetch_one(DATABASE.deref())
                .await
                .expect("could not fetch current live");

                let current_frontline = adjusted_frontline.load(Ordering::SeqCst);
                let increment = (current_frontline / 10).clamp(1, 5);

                let fallible = async {
                    let avg_mbps: f64 = average_mbps(alloc_group.clone()).await?;
                    let overload = avg_mbps / cfg.target_mbps;
                    if overload > 1.0 {
                        adjusted_frontline.fetch_add(increment, Ordering::SeqCst);
                        // adjusted_frontline.fetch_min(base_frontline * 4, Ordering::SeqCst);
                    } else if adjusted_frontline.load(Ordering::SeqCst) >= current_live as _ {
                        adjusted_frontline.fetch_sub(increment, Ordering::SeqCst);
                        adjusted_frontline.fetch_max(base_frontline, Ordering::SeqCst);
                    }
                    log::info!(
                        "adjusted frontline of {alloc_group} to {} on overload {overload}",
                        adjusted_frontline.load(Ordering::SeqCst)
                    );
                    anyhow::Ok(())
                };
                if let Err(err) = fallible.await {
                    log::warn!("could not adjust frontline: {:?}", err)
                } else {
                    (&mut timer).await;
                }
            }
        })
    };

    loop {
        if let Err(err) = loop_frontline_inner(&alloc_group, &cfg, adjusted_frontline.clone()).await
        {
            log::warn!("error: {:?}", err)
        }
        smol::Timer::after(Duration::from_secs(1)).await;
    }
}

async fn average_mbps(alloc_group: String) -> anyhow::Result<f64> {
    let addrs: Vec<(String,)> =
        sqlx::query_as("select ip_addr from bridges where alloc_group = $1")
            .bind(&alloc_group)
            .fetch_all(DATABASE.deref())
            .await?;

    let speed_measure = r#"
S1=$(for i in $(ls /sys/class/net | grep -v '^lo$'); do cat /sys/class/net/$i/statistics/rx_bytes; done | awk '{s+=$1} END{printf "%.0f", s}'); \
sleep 1; \
S2=$(for i in $(ls /sys/class/net | grep -v '^lo$'); do cat /sys/class/net/$i/statistics/rx_bytes; done | awk '{s+=$1} END{printf "%.0f", s}'); \
awk -v s1="$S1" -v s2="$S2" 'BEGIN {diff=s2-s1; printf "%.2f\n", diff*8/(1024*1024)}'
    "#;

    let mut speeds = Vec::new();

    for (addr,) in addrs {
        let resp = ssh_execute(&addr, speed_measure).await?;
        let resp: f64 = resp.trim().parse()?;
        speeds.push(resp);
    }

    if speeds.is_empty() {
        Ok(0.0)
    } else {
        log::debug!("averaging {alloc_group} from {:?}", speeds);
        let sum: f64 = speeds.iter().sum();
        let avg = sum / speeds.len() as f64;
        Ok(avg)
    }
}

#[allow(clippy::comparison_chain)]
async fn loop_frontline_inner(
    alloc_group: &str,
    _cfg: &GroupConfig,
    adjusted_frontline: Arc<AtomicUsize>,
) -> anyhow::Result<()> {
    let adjusted_frontline = adjusted_frontline.load(Ordering::SeqCst);
    // when not enough is in the frontline, move to frontline
    let (frontline_count,): (i64,) = sqlx::query_as(
        "select count(bridge_id) from bridges where (status = 'frontline' or status = 'blocked') and alloc_group = $1",
    )
    .bind(alloc_group)
    .fetch_one(DATABASE.deref())
    .await?;
    if frontline_count < adjusted_frontline as i64 {
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
    } else if frontline_count > adjusted_frontline as i64 {
        sqlx::query("delete from bridges where bridge_id in (select bridge_id from bridges where status = 'frontline' and alloc_group = $1 order by change_time limit 1)").bind(alloc_group).execute(DATABASE.deref()).await?;
    }
    Ok(())
}
