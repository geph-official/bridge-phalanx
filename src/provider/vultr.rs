use std::{collections::HashMap, time::Duration};

use anyhow::Context;
use isahc::AsyncReadResponseExt;
use serde::{Deserialize, Serialize};

use crate::config::VultrConfig;

use super::Provider;

pub struct VultrProvider {
    client: isahc::HttpClient,
    cfg: VultrConfig,
}

impl VultrProvider {
    /// Create a new Vultr-based provider.
    pub fn new(cfg: VultrConfig) -> Self {
        let client = isahc::HttpClientBuilder::new()
            .default_headers(&[
                ("Authorization", format!("Bearer {}", cfg.api_key).as_str()),
                ("Content-Type", "application/json"),
            ])
            .build()
            .unwrap();
        Self { client, cfg }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CreateServerArgs {
    region: String,
    plan: String,
    os_id: u32,
    label: String,

    sshkey_id: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct ServerDescriptor {
    id: String,
    label: String,
    status: String,
    #[serde(default)]
    main_ip: Option<String>,
}

impl Provider for VultrProvider {
    fn create_server(&self, id: &str) -> smol::Task<anyhow::Result<String>> {
        let id = id.to_string();
        let cfg = self.cfg.clone();
        let client = self.client.clone();
        smol::spawn(async move {
            let req = CreateServerArgs {
                label: vultrify_id(&id),

                region: cfg.region.clone(),
                plan: cfg.plan.clone(),
                os_id: cfg.os_id,
                sshkey_id: vec![cfg.sshkey_id.clone()],
            };
            let mut resp = client
                .post_async(
                    "https://api.vultr.com/v2/instances",
                    serde_json::to_vec(&req)?,
                )
                .await?;
            if !resp.status().is_success() {
                let r = resp.text().await?;
                anyhow::bail!("non-success while creating: {:?} {r}", resp.status())
            }
            // wait for the server to appear with a proper IP address
            loop {
                if let Some(server) = list_all(client.clone()).await?.into_iter().find(|server| {
                    server.label == vultrify_id(&id)
                        && server.main_ip.is_some()
                        && server.status == "active"
                }) {
                    return Ok(server.main_ip.unwrap());
                }
                smol::Timer::after(Duration::from_secs(1)).await;
            }
        })
    }

    fn retain_by_id(
        &self,
        _pred: Box<dyn Fn(&str) -> bool + Send + 'static>,
    ) -> smol::Task<anyhow::Result<()>> {
        smol::spawn(async move {
            log::warn!("vultr no delete yet");
            Ok(())
        })
    }
}

fn vultrify_id(id: &str) -> String {
    format!("vultr-phalanx-{id}")
}

/// List all the servers.
async fn list_all(client: isahc::HttpClient) -> anyhow::Result<Vec<ServerDescriptor>> {
    #[derive(Clone, Debug, Deserialize)]
    struct Resp {
        instances: Vec<ServerDescriptor>,
    }
    let haha: Resp = client
        .get_async("https://api.vultr.com/v2/instances")
        .await?
        .json()
        .await
        .context("could not decode list")?;
    Ok(haha.instances)
}
