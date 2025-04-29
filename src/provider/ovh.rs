use std::collections::BTreeMap;

use crate::provider::{system, wait_until_reachable};

use super::Provider;
use anyhow::Context;
use async_compat::CompatExt;
use async_trait::async_trait;
use openstack::waiter::Waiter;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OvhConfig {
    env_variables: BTreeMap<String, String>,

    flavor: String,
    network: String,
    image: String,
    keypair_name: String,
}

pub struct OvhProvider {
    cfg: OvhConfig,
}

impl OvhProvider {
    pub fn new(cfg: OvhConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Provider for OvhProvider {
    /// Creates a new server, returning an IP address reachable through SSH port 22 and "root".
    async fn create_server(&self, name: &str) -> anyhow::Result<String> {
        // Horrifying hax: set the env variables here lol
        for (k, v) in self.cfg.env_variables.iter() {
            std::env::set_var(k, v);
        }

        let os = openstack::Cloud::from_env()
            .compat()
            .await
            .context("Failed to create a Cloud object from the environment")?;

        log::info!("Creating OVH server...");
        let config = self.cfg.clone();
        let waiter = os
            .new_server(name, config.flavor)
            .with_image(config.image)
            .with_network(config.network)
            .with_keypair(config.keypair_name)
            .create()
            .await
            .context("Failed to create OVH server")?;
        {
            let current = waiter.current_state();
            log::info!(
                "ID = {}, Name = {}, Status = {:?}, Power = {:?}, Flavor = {:?}",
                current.id(),
                current.name(),
                current.status(),
                current.power_state(),
                current.flavor(),
            );
        }

        let server = waiter
            .wait()
            .await
            .context("Server did not reach ACTIVE state")?;
        log::info!(
            "Successfully created server -- ID = {}, Name = {}, Status = {:?}, Power = {:?}",
            server.id(),
            server.name(),
            server.status(),
            server.power_state()
        );
        let ipv4 = server
            .addresses()
            .values()
            .flat_map(|val| val.iter())
            .find_map(|addr| {
                if addr.addr.is_ipv4() {
                    Some(addr.addr)
                } else {
                    None
                }
            })
            .context("no ipv4 address?!?!?!")?;
        wait_until_reachable(&ipv4.to_string()).await;
        // enable root login
        system(&dbg!(format!("ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null debian@{ipv4} sudo cp /home/debian/.ssh/authorized_keys /root/.ssh/authorized_keys"))).await?;
        log::debug!("ENABLED ROOT ACCESS FOR OVH {ipv4}");

        Ok(ipv4.to_string())
    }

    /// Retains only the servers that match the given predicate.
    async fn retain_by_id(
        &self,
        pred: Box<dyn Fn(String) -> bool + Send + 'static>,
    ) -> anyhow::Result<()> {
        for (k, v) in self.cfg.env_variables.iter() {
            std::env::set_var(k, v);
        }

        let os = openstack::Cloud::from_env()
            .await
            .context("Failed to create a Cloud object from the environment")?;

        let servers = os.list_servers().await?;
        for server in servers.iter() {
            let name = server.name();
            if !pred(name.clone()) {
                log::info!("about to deleting server: {:?}", name);
                server.clone().delete().await?;
                log::info!("successfully deleted server: {:?}", name);
            }
        }

        Ok(())
    }
}
