use std::{sync::LazyLock, time::Duration};

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use dashmap::DashSet;
use isahc::{http::StatusCode, AsyncReadResponseExt, HttpClient};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{wait_until_reachable, Provider};

const API: &str = "https://api.serverspace.io/api/v1";
static CREATING: LazyLock<DashSet<String>> = LazyLock::new(DashSet::new);

/* ---------- user-supplied config ---------- */
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerSpaceConfig {
    pub api_key: String,
    pub location_id: String,
    pub image_id: String,
    pub cpu: u32,
    pub ram_mb: u32,
    pub boot_size_mb: u32,
    pub bandwidth_mbps: u32,
    pub ssh_key_ids: Vec<i32>,
}

/* ---------- provider ---------- */
pub struct ServerSpaceProvider {
    client: HttpClient,
    cfg: ServerSpaceConfig,
}

impl ServerSpaceProvider {
    pub fn new(cfg: ServerSpaceConfig) -> Self {
        let client = HttpClient::builder()
            .default_header("Content-Type", "application/json")
            .default_header("X-API-KEY", &cfg.api_key)
            .build()
            .unwrap();
        Self { client, cfg }
    }

    /* ----- helpers that work with raw JSON ----- */

    async fn json_get(&self, url: &str) -> anyhow::Result<Value> {
        let mut r = self.client.get_async(url).await?;
        if r.status() != StatusCode::OK {
            anyhow::bail!("GET {} failed: {}", url, r.text().await?)
        }
        Ok(r.json().await?)
    }

    async fn json_del(&self, url: &str) -> anyhow::Result<()> {
        let mut r = self.client.delete_async(url).await?;
        if r.status() != StatusCode::OK {
            anyhow::bail!("DELETE {} failed: {}", url, r.text().await?)
        }
        Ok(())
    }

    async fn poll_task(&self, task_id: &str) -> anyhow::Result<()> {
        loop {
            let v = self.json_get(&format!("{API}/tasks/{task_id}")).await?;
            match v["task"]["is_completed"].as_str() {
                Some("Completed") => return Ok(()),
                Some("Failed") => anyhow::bail!("task {task_id} failed"),
                _ => {
                    smol::Timer::after(Duration::from_secs(3)).await;
                }
            }
        }
    }

    /* ----- anything that pokes the “servers” collection ----- */

    async fn list_servers(&self) -> anyhow::Result<Vec<Value>> {
        let v = self.json_get(&format!("{API}/servers")).await?;
        Ok(v["servers"].as_array().unwrap_or(&vec![]).clone())
    }

    async fn server(&self, id: &str) -> anyhow::Result<Value> {
        self.json_get(&format!("{API}/servers/{id}")).await
    }

    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.json_del(&format!("{API}/servers/{id}")).await
    }
}

/* ---------- Provider impl ---------- */
#[async_trait]
impl Provider for ServerSpaceProvider {
    async fn create_server(&self, id: &str) -> anyhow::Result<String> {
        let label = format!("bridge-{id}");
        let body = json!({
            "location_id":  self.cfg.location_id,
            "image_id":     self.cfg.image_id,
            "cpu":          self.cfg.cpu,
            "ram_mb":       self.cfg.ram_mb,
            "volumes": [{ "name": "boot", "size_mb": self.cfg.boot_size_mb }],
            "networks": [{ "bandwidth_mbps": self.cfg.bandwidth_mbps }],
            "name":         label,
            "ssh_key_ids":  self.cfg.ssh_key_ids,
        });

        // fire-and-forget
        let mut resp = self
            .client
            .post_async(format!("{API}/servers"), body.to_string().into_bytes())
            .await?;
        if resp.status() != StatusCode::OK {
            anyhow::bail!("create failed: {}", resp.text().await?)
        }
        let v: Value = resp.json().await?;
        let task_id = v["task_id"]
            .as_str()
            .context("missing task_id in create-server response")?;
        let server_id = v
            .get("server_id")
            .and_then(|x| x.as_str())
            // some clusters still return only task_id; in that case
            // task-details.embed.server_id is present once the task is done
            .unwrap_or_default()
            .to_string();

        CREATING.insert(server_id.clone());
        scopeguard::defer! { CREATING.remove(&server_id); }

        // wait for provisioning
        self.poll_task(task_id).await?;

        // if the first response lacked server_id, read it from the finished task
        let sid = if server_id.is_empty() {
            let t = self.json_get(&format!("{API}/tasks/{task_id}")).await?;
            t["task"]["server_id"]
                .as_str()
                .ok_or_else(|| anyhow!("task finished but server_id not found"))?
                .to_string()
        } else {
            server_id.clone()
        };

        // wait until the VM is “Active” and has a public IP
        loop {
            let s = self.server(&sid).await?;
            if s["state"] == "Active" {
                if let Some(ip) = s["nics"].as_array().and_then(|a| {
                    a.iter()
                        .find(|n| n["network_type"] == "PublicShared")
                        .and_then(|n| n["ip_address"].as_str())
                }) {
                    wait_until_reachable(ip).await;
                    return Ok(ip.to_owned());
                }
            }
            smol::Timer::after(Duration::from_secs(2)).await;
        }
    }

    async fn retain_by_id(
        &self,
        pred: Box<dyn Fn(String) -> bool + Send + 'static>,
    ) -> anyhow::Result<()> {
        for s in self.list_servers().await? {
            let srv_id = s["id"].to_string();
            let label = s["name"].as_str().unwrap_or("").to_string();
            let short = label.strip_prefix("bridge-").unwrap_or(&label).to_string();

            if !pred(short)
                && !CREATING.contains(&srv_id)
                && s["nics"].as_array().map_or(false, |n| {
                    n.iter().any(|n| n["network_type"] == "PublicShared")
                })
            {
                log::debug!("Serverspace: deleting {srv_id} ({label})");
                self.delete(&srv_id).await?;
            }
        }
        Ok(())
    }
}
