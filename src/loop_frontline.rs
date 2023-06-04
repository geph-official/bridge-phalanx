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
        let provider = provider.clone();
        let alloc_group = alloc_group.clone();
        smol::spawn(async move {
            let mut timer = smol::Timer::interval(Duration::from_secs(600));
            loop {
                let current_frontline = adjusted_frontline.load(Ordering::SeqCst);
                let increment = (current_frontline / 10).max(1).min(5);
                let fallible = async {
                    let overload = provider.overload().await?;
                    if overload > 1.0 {
                        adjusted_frontline.fetch_add(increment, Ordering::SeqCst);
                        // adjusted_frontline.fetch_min(base_frontline * 4, Ordering::SeqCst);
                    } else {
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

#[allow(clippy::comparison_chain)]
async fn loop_frontline_inner(
    alloc_group: &str,
    cfg: &GroupConfig,
    adjusted_frontline: Arc<AtomicUsize>,
) -> anyhow::Result<()> {
    let adjusted_frontline = adjusted_frontline.load(Ordering::SeqCst);
    // when not enough is in the frontline, move to frontline
    let (frontline_count,): (i64,) = sqlx::query_as(
        "select count(bridge_id) from bridges where status = 'frontline' and alloc_group = $1",
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
