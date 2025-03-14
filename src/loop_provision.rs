use std::{ops::Deref, sync::Arc, time::Duration};

use anyhow::Context;
use futures_util::{stream::FuturesUnordered, StreamExt};

use rand::Rng;
use smol_timeout::TimeoutExt;

use crate::{
    config::{
        GroupConfig, Service, CONFIG, EARENDIL_GIST, GEPH4_GIST, GEPH5_GIST, GEPH5_EXIT_SCRIPT, LIMIT_BANDWIDTH_GIST,
    },
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
            let cachebust = rand::thread_rng().gen::<u64>();
            if cfg.services.contains(&Service::Geph4) {
                ssh_execute(&addr, &format!("wget -qO- {}?cachebust={cachebust} | env AGROUP={remote_alloc_group} BSECRET={bridge_secret} sh", GEPH4_GIST)).await?;
            }
            if cfg.services.contains(&Service::Geph5) {
                ssh_execute(&addr, &format!("wget -qO- {}?cachebust={cachebust} | env AGROUP={remote_alloc_group} BSECRET={bridge_secret} sh", GEPH5_GIST)).await?;
            }
            if cfg.services.contains(&Service::Earendil) {
                ssh_execute(&addr, &format!("wget -qO- {}?cachebust={cachebust} | env AGROUP={remote_alloc_group} BSECRET={bridge_secret} sh", EARENDIL_GIST)).await?;
            }
            if cfg.services.contains(&Service::Geph5Exit) {
                // Run the original setup script first
                ssh_execute(&addr, &format!("wget -q -O /tmp/script.sh {} && AUTH_TOKEN=fc9d0d668165135a18f6fa42c82a7971c43b7d07 bash /tmp/script.sh", GEPH5_EXIT_SCRIPT)).await?;
                
                // After the script has run, override the country, city, and total_ratelimit if specified
                if cfg.exit_country.is_some() || cfg.exit_city.is_some() || cfg.exit_total_ratelimit.is_some() {
                    // Create a script to update the config file
                    let update_config_commands = format!(
                        r#"
                        # Backup the original config
                        cp /etc/geph5-exit/config.yaml /etc/geph5-exit/config.yaml.orig
                        
                        # Update the config file with overridden values
                        {}
                        {}
                        {}
                        
                        # Restart the service to apply changes
                        systemctl restart geph5-exit
                        "#,
                        cfg.exit_country.as_ref().map_or(String::new(), |country| 
                            format!("sed -i 's/^country: .*/country: {}/' /etc/geph5-exit/config.yaml", country)),
                        cfg.exit_city.as_ref().map_or(String::new(), |city| 
                            format!("sed -i 's/^city: .*/city: {}/' /etc/geph5-exit/config.yaml", city)),
                        cfg.exit_total_ratelimit.as_ref().map_or(String::new(), |limit| 
                            format!("echo 'total_ratelimit: {}' >> /etc/geph5-exit/config.yaml", limit))
                    );
                    
                    // Execute the config update script
                    ssh_execute(&addr, &format!("bash -c \"{}\"", update_config_commands)).await?;
                }
            }
            if let Some(max_bandwidth_gb) = cfg.max_bandwidth_gb {
                ssh_execute(&addr, &format!("wget -qO- {}?cachebust={cachebust} | env TRAFFIC_LIMIT_GB={max_bandwidth_gb} sh", LIMIT_BANDWIDTH_GIST)).await?;
            }
            // ssh_execute(&addr, &format!("shutdown -h +{}", (cfg.max_lifetime_hr / 60.0) as u64)).await?;
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