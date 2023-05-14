use std::{
    collections::HashMap,
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use acidjson::AcidJson;
use anyhow::Context;

use async_trait::async_trait;
use isahc::{AsyncReadResponseExt, Request, RequestExt};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use smol::io::AsyncReadExt;

use crate::provider::wait_until_reachable;

use super::Provider;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScalewayConfig {
    pub secret_key: String,
    pub zone: String,
    pub project_id: String,
    pub commercial_type: String,
    pub image: String,
}

pub struct ScalewayProvider {
    cfg: ScalewayConfig,
}

impl ScalewayProvider {
    pub fn new(cfg: ScalewayConfig) -> Self {
        Self { cfg }
    }
}

static RECENT_IDS: Lazy<Mutex<HashMap<String, Instant>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn add_recent(s: &str) {
    RECENT_IDS.lock().insert(s.to_string(), Instant::now());
}

fn check_recent(s: &str) -> bool {
    let mut lst = RECENT_IDS.lock();
    lst.retain(|_k, v| v.elapsed().as_secs_f64() < 600.0);
    lst.contains_key(s)
}

static USED_IPS: Lazy<AcidJson<HashMap<String, u64>>> = Lazy::new(|| {
    let fname = "/tmp/scw-ips.json";
    if !Path::new(fname).exists() {
        std::fs::write(fname, "{}").unwrap();
    }
    AcidJson::open(Path::new(fname)).unwrap()
});

#[async_trait]
impl Provider for ScalewayProvider {
    async fn create_server(&self, phalanx_id: &str) -> anyhow::Result<String> {
        add_recent(phalanx_id);
        let create_server_req = json!({
            "name": phalanx_id,
            "project": self.cfg.project_id,
            "commercial_type": self.cfg.commercial_type,
            "image": self.cfg.image,
            "enable_ipv6": false,
            "dynamic_ip_required": true,
        });

        let id = phalanx_id.to_string();
        let cfg = self.cfg.clone();
        let resp = Request::post(format!(
            "https://api.scaleway.com/instance/v1/zones/{}/servers",
            cfg.zone
        ))
        .header("content-type", "application/json")
        .header("X-Auth-Token", &cfg.secret_key)
        .body(create_server_req.to_string())?
        .send_async()
        .await?;

        if resp.status() != 200 && resp.status() != 201 {
            let status = resp.status();
            let mut err_body = String::new();
            resp.into_body().read_to_string(&mut err_body).await?;
            anyhow::bail!("status {}, body {}", status, err_body);
        }

        let mut rr = String::new();
        resp.into_body().read_to_string(&mut rr).await?;
        let resp: Value =
            serde_json::from_str(&rr).context("cannot parse server resp for create")?;
        let scaleway_id = resp["server"]["id"]
            .as_str()
            .context("invalid json in response")?
            .to_string();
        log::debug!("server created {id}");

        perform_action(&cfg, &scaleway_id, "poweron")
            .await
            .context("cannot turn on server")?;
        log::debug!("turned on {id}");
        loop {
            let ip_addr = get_server(&cfg, &scaleway_id)
                .await
                .context("cannot get server")?["server"]["public_ip"]["address"]
                .as_str()
                .map(|s| s.to_string());
            if let Some(ip_addr) = ip_addr {
                if USED_IPS
                    .write()
                    .insert(
                        ip_addr.clone(),
                        SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
                    )
                    .is_some()
                {
                    anyhow::bail!("already seen IP {ip_addr}");
                }

                log::debug!("got IP address {id}: {ip_addr}");
                wait_until_reachable(&ip_addr).await;
                log::debug!("fully done {id}");

                return Ok(ip_addr.to_string());
            }
        }
    }

    async fn retain_by_id(
        &self,
        pred: Box<dyn Fn(String) -> bool + Send + 'static>,
    ) -> anyhow::Result<()> {
        let cfg = self.cfg.clone();
        let base_url = format!(
            "https://api.scaleway.com/instance/v1/zones/{}/servers",
            cfg.zone
        );

        for current_page in 1.. {
            let url = format!("{}?per_page=10&page={}", base_url, current_page);
            let resp = Request::get(&url)
                .header("X-Auth-Token", &cfg.secret_key)
                .body("")?
                .send_async()
                .await?;
            let mut body = String::new();
            resp.into_body().read_to_string(&mut body).await?;
            let server_list: Value = serde_json::from_str(&body)?;

            let servers = server_list["servers"]
                .as_array()
                .ok_or(anyhow::Error::msg("No servers found"))?;

            if servers.is_empty() {
                break;
            }

            for server in servers.iter() {
                let server_id = server["id"]
                    .as_str()
                    .ok_or(anyhow::Error::msg("No server id found"))?;
                let server_name = server["name"]
                    .as_str()
                    .ok_or(anyhow::Error::msg("No server name found"))?;

                if !pred(server_name.to_string()) && !check_recent(server_name) {
                    log::debug!("SCALEWAY DELETING {}", server_name);
                    delete_server(&cfg, server_id).await?;
                }
            }
        }

        Ok(())
    }
}

async fn get_server(cfg: &ScalewayConfig, scw_server_id: &str) -> anyhow::Result<Value> {
    let mut response = Request::get(format!(
        "https://api.scaleway.com/instance/v1/zones/{}/servers/{}",
        cfg.zone, scw_server_id
    ))
    .header("X-Auth-Token", &cfg.secret_key)
    .body("")?
    .send_async()
    .await?;

    let body = response.text().await?;
    let parsed_response = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("Failed to parse response: {}\n{}", e.to_string(), body))?;

    Ok(parsed_response)
}

async fn perform_action(
    cfg: &ScalewayConfig,
    scw_server_id: &str,
    action: &str,
) -> anyhow::Result<()> {
    let action_req = json!({
        "action": action.to_string(),
    });

    let request = Request::post(format!(
        "https://api.scaleway.com/instance/v1/zones/{}/servers/{}/action",
        cfg.zone, scw_server_id
    ))
    .header("X-Auth-Token", &cfg.secret_key)
    .header("Content-Type", "application/json")
    .body(action_req.to_string())?;

    let mut response = isahc::send_async(request).await?;

    if response.status() != 200 && response.status() != 202 {
        let body = response
            .text()
            .await
            .context("Failed to read response body")?;
        anyhow::bail!("Request failed with status {}: {}", response.status(), body);
    }

    Ok(())
}

async fn delete_server(cfg: &ScalewayConfig, scw_server_id: &str) -> anyhow::Result<()> {
    if let Err(err) = perform_action(cfg, scw_server_id, "terminate").await {
        log::warn!(
            "could not terminate ({:?}), deleting instead {}",
            err,
            scw_server_id
        );
        log::debug!("deleting {}", scw_server_id);
        let request = Request::delete(format!(
            "https://api.scaleway.com/instance/v1/zones/{}/servers/{}",
            cfg.zone, scw_server_id
        ))
        .header("X-Auth-Token", &cfg.secret_key)
        .header("Content-Type", "application/json")
        .body("")?;

        let mut response = isahc::send_async(request).await?;

        if response.status() != 200 && response.status() != 204 {
            let body = response
                .text()
                .await
                .context("Failed to read response body")?;
            anyhow::bail!("delete failed with status {}: {}", response.status(), body);
        }
    }
    Ok(())
}
