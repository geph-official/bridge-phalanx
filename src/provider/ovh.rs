use super::Provider;
use async_trait::async_trait;
use openstack::waiter::Waiter;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OVHConfig {
    flavor: String,
    network: String,
    image: String,
    keypair_name: String,
}

pub struct OVHProvider {
    cfg: OVHConfig,
}

impl OVHProvider {
    pub fn new(cfg: OVHConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Provider for OVHProvider {
    /// Creates a new server, returning an IP address reachable through SSH port 22 and "root".
    async fn create_server(&self, name: &str) -> anyhow::Result<String> {
        // NOTE: Requires us to `source` a config file (e.g. openrc.sh) for the cloud project's OpenStack user.
        // Alternatively, we can also instantiate the `Cloud` object from a local config YAML.
        let os = openstack::Cloud::from_env()
            .await
            .expect("Failed to create a Cloud object from the environment");

        log::info!("Creating OVH server...");
        let config = self.cfg.clone();
        let waiter = os
            .new_server(name, config.flavor)
            .with_image(config.image)
            .with_network(config.network)
            .with_keypair(config.keypair_name)
            .create()
            .await
            .expect("Failed to create OVH server");
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
            .expect("Server did not reach ACTIVE state");
        log::info!(
            "Successfully created server -- ID = {}, Name = {}, Status = {:?}, Power = {:?}",
            server.id(),
            server.name(),
            server.status(),
            server.power_state()
        );
        let ipv4 = server
            .access_ipv4()
            .expect("newly created server has no IPv4 address");
        Ok(ipv4.to_string())
    }

    /// Retains only the servers that match the given predicate.
    async fn retain_by_id(
        &self,
        pred: Box<dyn Fn(String) -> bool + Send + 'static>,
    ) -> anyhow::Result<()> {
        let os = openstack::Cloud::from_env()
            .await
            .expect("Failed to create a Cloud object from the environment");

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
