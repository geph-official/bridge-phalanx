use std::sync::Arc;

use config::{ProviderConfig, CONFIG};
use loop_frontline::loop_frontline;
use loop_gfw::loop_gfw;
use loop_onoff::loop_onoff;
use loop_provision::loop_provision;
use loop_prune::loop_prune;
use provider::{lightsail::LightsailProvider, vultr::VultrProvider, Provider};

mod config;
mod database;
mod loop_frontline;
mod loop_gfw;
mod loop_onoff;
mod loop_provision;
mod loop_prune;
mod provider;
mod ssh;

fn main() {
    env_logger::init();
    smol::block_on(async {
        smol::spawn(loop_onoff()).detach();
        smol::spawn(loop_gfw()).detach();
        smol::spawn(loop_prune()).detach();

        // for every provider, start the right loops
        for (group, group_cfg) in CONFIG.groups.iter() {
            let provider: Arc<dyn Provider> = match &group_cfg.provider {
                ProviderConfig::Lightsail(cfg) => Arc::new(LightsailProvider::new(cfg.clone())),
                ProviderConfig::Vultr(cfg) => Arc::new(VultrProvider::new(cfg.clone())),
            };
            smol::spawn(loop_provision(
                group.to_string(),
                group_cfg.clone(),
                provider,
            ))
            .detach();
            smol::spawn(loop_frontline(group.to_string(), group_cfg.clone())).detach();
        }

        smol::future::pending().await
    })
}
