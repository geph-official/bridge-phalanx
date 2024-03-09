use std::{ops::Deref, sync::Arc, time::Duration};

use anyhow::Context;
use futures_util::{stream::FuturesUnordered, StreamExt};

use rand::Rng;
use smol_timeout::TimeoutExt;

use crate::{
    config::{GroupConfig, CONFIG},
    database::{BridgeInfo, DATABASE},
    provider::Provider,
    ssh::ssh_execute,
};

pub async fn loop_provision(alloc_group: String, cfg: GroupConfig, provider: Arc<dyn Provider>) {
    loop {
        let secs = rand::thread_rng().gen::<f64>() * 5.0;
        smol::Timer::after(Duration::from_secs_f64(secs)).await;
        log::info!("***** provision once {alloc_group} *****");
        if let Err(err) = loop_provision_once(&alloc_group, &cfg, provider.as_ref()).await {
            log::warn!("{alloc_group} error: {:?}", err)
        }
    }
}

async fn loop_provision_once(
    alloc_group: &str,
    cfg: &GroupConfig,
    provider: &dyn Provider,
) -> anyhow::Result<()> {
    async {
    {
        let bridges: Vec<BridgeInfo> = sqlx::query_as("select * from bridges")
            .fetch_all(DATABASE.deref())
            .await?;
        provider
            .retain_by_id(Box::new(move |id| {
                bridges.iter().any(|b| b.bridge_id == id)
            }))
            .await?;
    }



    let (reserve_count,): (i64,) = sqlx::query_as(
        "select count(bridge_id) from bridges where status = 'reserve' and alloc_group = $1",
    )
    .bind(alloc_group)
    .fetch_one(DATABASE.deref())
    .await?;
    if reserve_count < cfg.reserve as i64 {
        log::debug!("**** {alloc_group} REPLENISH {} -> {} ****", reserve_count, cfg.reserve);
        let mut tasks = FuturesUnordered::new();
        for _ in 0..((cfg.reserve as i64) - reserve_count).min(64) {
            tasks.push(async  {

            let id = new_id();
            let addr = provider.create_server(&id).await.context("cannot create more")?;
            let remote_alloc_group = cfg.override_group.as_deref().unwrap_or(alloc_group);
            // set into reserve status
            let bridge_secret = &CONFIG.bridge_secret;
            ssh_execute(&addr, &format!("wget -qO- https://gist.githubusercontent.com/nullchinchilla/ecf752dfb3ff33635d1f6487b5a87531/raw/c9de7443bcb6e1fc03f3bc7ee91e101eded9ac36/deploy-bridge-new.sh | env AGROUP={remote_alloc_group} BSECRET={bridge_secret} sh")).await?;
            sqlx::query("insert into bridges (bridge_id, ip_addr, alloc_group, status, change_time) values ($1, $2, $3, $4, NOW())").bind(id).bind(addr).bind(alloc_group).bind("reserve").execute(DATABASE.deref()).await?;
            anyhow::Ok(())
            });
        }
        while let Some(next) = tasks.next().await {
            next?;
        }
    }

    anyhow::Ok(())
}.timeout(Duration::from_secs(3600)).await.ok_or_else(|| anyhow::anyhow!("timeout"))?
}

fn new_id() -> String {
    format!(
        "{}-{}-{}-{}-{}",
        eff_wordlist::large::random_word(),
        eff_wordlist::large::random_word(),
        eff_wordlist::large::random_word(),
        eff_wordlist::large::random_word(),
        eff_wordlist::large::random_word()
    )
}
