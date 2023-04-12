use std::{collections::HashMap, time::Instant};

use anyhow::Context;
use isahc::{AsyncReadResponseExt, Request, RequestExt};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use smol::io::AsyncReadExt;

use crate::{config::ScalewayConfig, provider::wait_until_reachable};

use super::Provider;

pub struct ScalewayProvider {
    cfg: ScalewayConfig,
}

impl ScalewayProvider {
    pub fn new(cfg: ScalewayConfig) -> Self {
        Self { cfg }
    }
}

static RECENT_LIST: Lazy<Mutex<HashMap<String, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn add_recent(s: &str) {
    RECENT_LIST.lock().insert(s.to_string(), Instant::now());
}

fn check_recent(s: &str) -> bool {
    let mut lst = RECENT_LIST.lock();
    lst.retain(|k, v| v.elapsed().as_secs_f64() < 120.0);
    lst.contains_key(s)
}

impl Provider for ScalewayProvider {
    fn create_server(&self, id: &str) -> smol::Task<anyhow::Result<String>> {
        add_recent(id);
        let create_server_req = json!({
            "name": id,
            "project": self.cfg.project_id,
            "commercial_type": self.cfg.commercial_type,
            "image": self.cfg.image,
            "enable_ipv6": false,
            "dynamic_ip_required": false,
        });

        let id = id.to_string();
        let cfg = self.cfg.clone();
        smol::spawn(async move {
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
            let server_id = resp["server"]["id"]
                .as_str()
                .context("invalid json in response")?
                .to_string();
            log::debug!("server created {id}");

            perform_action(&cfg, &server_id, "poweron")
                .await
                .context("cannot turn on server")?;
            log::debug!("turned on {id}");

            loop {
                let server_resp = get_server(&cfg, &server_id)
                    .await
                    .context("cannot get server")?;
                if let Some(addr) = server_resp["server"]["public_ip"]["address"].as_str() {
                    let ip_addr = addr.to_string();
                    log::debug!("got IP address {id}: {ip_addr}");
                    wait_until_reachable(&ip_addr).await;
                    log::debug!("fully done {id}");

                    return Ok(ip_addr);
                }
            }
        })
    }

    fn retain_by_id(
        &self,
        pred: Box<dyn Fn(&str) -> bool + Send + 'static>,
    ) -> smol::Task<anyhow::Result<()>> {
        let cfg = self.cfg.clone();
        smol::spawn(async move {
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
                    // log::debug!("checking {server_name}");
                    if !pred(server_name) && !check_recent(server_name) {
                        log::debug!("SCALEWAY DELETING {}", server_name);
                        delete_server(&cfg, server_id).await?;
                    }
                }
            }

            Ok(())
        })
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
