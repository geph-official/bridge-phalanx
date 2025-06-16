use async_trait::async_trait;
use isahc::{AsyncReadResponseExt, Request, RequestExt};
use serde::{Deserialize, Serialize};
use smol::io::AsyncReadExt;

use crate::{
    id::new_id,
    provider::{wait_until_reachable, CreatedServer},
};

use super::Provider;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HetznerConfig {
    pub api_token: String,
    pub server_type: String,
    pub location: String,
    pub image: String,
    pub sshkey_id: String,
}

pub struct HetznerProvider {
    cfg: HetznerConfig,
}

impl HetznerProvider {
    pub fn new(cfg: HetznerConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Provider for HetznerProvider {
    async fn create_server(&self) -> anyhow::Result<CreatedServer> {
        let id = new_id();
        #[derive(Serialize)]
        struct CreateServerReq {
            name: String,
            server_type: String,
            image: String,
            location: String,
            ssh_keys: Vec<String>,
        }

        let id = id.to_string();
        let cfg = self.cfg.clone();
        let mut resp = Request::post("https://api.hetzner.cloud/v1/servers")
            .header("content-type", "application/json")
            .header("Authorization", format!("Bearer {}", cfg.api_token))
            .body(serde_json::to_vec(&CreateServerReq {
                name: id.clone(),
                server_type: cfg.server_type.clone(),
                image: cfg.image.clone(),
                location: cfg.location.clone(),
                ssh_keys: vec![cfg.sshkey_id.clone()],
            })?)?
            .send_async()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("failed to create server: {}", resp.text().await?);
        }

        let body: serde_json::Value = resp.json().await?;
        let val = body["server"]["public_net"]["ipv4"]["ip"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Failed to extract IPv4 address from response"))?;

        wait_until_reachable(val).await;
        Ok(CreatedServer {
            id,
            ip_addr: val.to_string(),
        })
    }

    async fn retain_by_id(
        &self,
        pred: Box<dyn Fn(String) -> bool + Send + 'static>,
    ) -> anyhow::Result<()> {
        let cfg = self.cfg.clone();
        // List all available servers
        let resp = Request::get("https://api.hetzner.cloud/v1/servers")
            .header("Authorization", format!("Bearer {}", cfg.api_token))
            .body("")?
            .send_async()
            .await?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("Failed to list servers: {}", resp.status()));
        }

        let mut body = Vec::new();
        resp.into_body().read_to_end(&mut body).await?;
        let json: serde_json::Value = serde_json::from_slice(&body)?;

        // Filter the servers based on the given predicate
        let servers = json["servers"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Failed to parse server list"))?;

        for server in servers {
            let server_id_str = server["name"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_default();
            log::debug!("looking at {server_id_str}");
            if !(pred)(server_id_str.clone()) {
                log::debug!("DELETING {server_id_str}");
                // Delete the server if it doesn't match the predicate
                let srv_id: i32 = server["id"]
                    .as_i64()
                    .ok_or_else(|| anyhow::anyhow!("Failed to get server id"))?
                    as i32;

                let mut delete_resp =
                    Request::delete(format!("https://api.hetzner.cloud/v1/servers/{}", srv_id))
                        .header("Authorization", format!("Bearer {}", cfg.api_token))
                        .body("")?
                        .send_async()
                        .await?;

                if !delete_resp.status().is_success() {
                    return Err(anyhow::anyhow!(
                        "Failed to delete server: {} {}",
                        delete_resp.status(),
                        delete_resp.text().await?
                    ));
                }
            }
        }

        Ok(())
    }

    // Implement the retain_by_id method here
}
