use std::{ops::Deref, sync::Arc, time::Duration};

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
        log::info!("***** provision once {alloc_group} *****");
        if let Err(err) = loop_provision_once(&alloc_group, &cfg, provider.as_ref()).await {
            log::warn!("error: {:?}", err)
        }
        smol::Timer::after(Duration::from_secs(5)).await;
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

        let mut tasks = FuturesUnordered::new();
        for _ in 0..((cfg.reserve as i64) - reserve_count).min(64) {
            tasks.push(async  {

            let id = new_id();
            let addr = provider.create_server(&id).await?;
            // set into reserve status
            let bridge_secret = &CONFIG.bridge_secret;
            ssh_execute(&addr, &format!(" wget -qO- https://gist.githubusercontent.com/nullchinchilla/ecf752dfb3ff33635d1f6487b5a87531/raw/deploy-bridge-new.sh | env AGROUP={alloc_group} BSECRET={bridge_secret} sh")).await?;
            sqlx::query("insert into bridges (bridge_id, ip_addr, alloc_group, status, change_time) values ($1, $2, $3, $4, NOW())").bind(id).bind(addr).bind(alloc_group).bind("reserve").execute(DATABASE.deref()).await?;
            anyhow::Ok(())
            });
        }
        while let Some(next) = tasks.next().await {
            next?;
        }
    }

    anyhow::Ok(())
}.timeout(Duration::from_secs(300)).await.ok_or_else(|| anyhow::anyhow!("timeout"))?
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
