use anyhow::Context;
use async_trait::async_trait;
use isahc::{AsyncReadResponseExt, Request, RequestExt};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use smol::io::AsyncReadExt;

use crate::{provider::system, ssh::ssh_execute};

use super::{wait_until_reachable, Provider};
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OneCloudConfig {
    pub api_key: String,
    pub client_key: String,
    pub location_id: usize,
    pub instance_size: String,
    pub template: usize,
    pub ssh_key: String,
}

pub struct OneCloudProvider {
    cfg: OneCloudConfig,
}

impl OneCloudProvider {
    pub fn new(cfg: OneCloudConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Provider for OneCloudProvider {
    async fn create_server(&self, phalanx_id: &str) -> anyhow::Result<String> {
        let create_server_req = vec![
            ("hostname", phalanx_id.to_string()),
            ("location_id", self.cfg.location_id.to_string()),
            ("instance_size", self.cfg.instance_size.clone()),
            ("template", self.cfg.template.to_string()),
        ];

        let form_data = serde_urlencoded::to_string(create_server_req)?;

        let cfg = self.cfg.clone();
        let resp = Request::post("https://api.oneprovider.com/vm/create")
            .header("Api-Key", &cfg.api_key)
            .header("Client-Key", &cfg.client_key)
            .body(form_data)?
            .send_async()
            .await?;

        dbg!(&resp);

        if resp.status() != 200 {
            let status = resp.status();
            let mut err_body = String::new();
            resp.into_body().read_to_string(&mut err_body).await?;
            anyhow::bail!("status {}, body {}", status, err_body);
        }

        let mut rr = String::new();
        resp.into_body().read_to_string(&mut rr).await?;
        dbg!(&rr);
        let resp: Value =
            serde_json::from_str(&rr).context("cannot parse server resp for create")?;
        let ip_addr = resp["response"]["ip_address"]
            .as_str()
            .context("invalid json in response")?
            .to_string();
        log::debug!("server created {phalanx_id}");
        wait_until_reachable(&ip_addr).await;
        let password = resp["response"]["password"]
            .as_str()
            .context("no password")?;
        system(&format!("sshpass -p {password} ssh-copy-id -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@{ip_addr}")).await?;
        ssh_execute(&ip_addr, "apt update -y").await?;
        Ok(ip_addr)
    }

    async fn retain_by_id(
        &self,
        pred: Box<dyn Fn(String) -> bool + Send + 'static>,
    ) -> anyhow::Result<()> {
        let cfg = self.cfg.clone();
        let url = "https://api.oneprovider.com/vm/list";
        let mut resp = Request::get(url)
            .header("Api-Key", &cfg.api_key)
            .header("Client-Key", &cfg.client_key)
            .body("")?
            .send_async()
            .await?;

        let body = resp.text().await?;
        let server_list: Value = serde_json::from_str(&body)?;

        let servers = server_list["response"]["instances"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        for server in servers.iter() {
            let server_id = server["id"]
                .as_str()
                .ok_or(anyhow::Error::msg("No server id found"))?;
            let server_name = server["domain"]
                .as_str()
                .ok_or(anyhow::Error::msg("No server name found"))?;

            if !pred(server_name.to_string()) {
                log::debug!("ONEPROVIDER DELETING {}", server_name);
                delete_server(&cfg, server_id).await?;
            }
        }

        Ok(())
    }
}

async fn delete_server(cfg: &OneCloudConfig, phalanx_id: &str) -> anyhow::Result<()> {
    let delete_server_req = vec![("vm_id", phalanx_id), ("confirm_close", "true")];

    let form_data = serde_urlencoded::to_string(delete_server_req)?;

    let mut resp = Request::post("https://api.oneprovider.com/vm/destroy")
        .header("Api-Key", &cfg.api_key)
        .header("Client-Key", &cfg.client_key)
        .body(form_data)?
        .send_async()
        .await?;

    if resp.status() != 200 {
        let status = resp.status();
        let body = resp.text().await?;
        anyhow::bail!("delete failed with status {}: {}", status, body);
    }

    log::debug!("server deleted {phalanx_id}");
    Ok(())
}
