use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::provider::{system, wait_until_reachable};

use super::Provider;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LightsailConfig {
    pub access_key_id: String,
    pub secret_access_key: String,

    pub region: String,
    pub availability_zone: String,

    pub bundle_id: String,
    pub key_pair_name: String,

    pub target_cpu_usage: f64,
}

pub struct LightsailProvider {
    cfg: LightsailConfig,
}

impl LightsailProvider {
    /// Creates a new Lightsail provider.
    pub fn new(cfg: LightsailConfig) -> Self {
        Self { cfg }
    }

    /// Query the burst capacity percentage of a particular server.
    async fn burst_capacity_percent(&self, aws_name: &str) -> anyhow::Result<f64> {
        let LightsailConfig {
            access_key_id,
            secret_access_key,
            region,
            availability_zone: _,
            bundle_id: _,
            key_pair_name: _,
            target_cpu_usage: _,
        } = &self.cfg;
        let result = system(&format!("AWS_ACCESS_KEY_ID={access_key_id} AWS_SECRET_ACCESS_KEY={secret_access_key} AWS_DEFAULT_REGION={region} aws lightsail get-instance-metric-data --instance-name {aws_name} --metric-name BurstCapacityPercentage --unit Percent --start-time $(date -u -d '1 hour ago' +%FT%TZ) --end-time $(date -u +%FT%TZ) --period 600 --statistics Average")).await?;
        let result: serde_json::Value = serde_json::from_str(&result)?;
        let data = result["metricData"]
            .as_array()
            .context("metricData not an array")?
            .iter()
            .max_by_key(|v| v["timestamp"].as_f64().unwrap_or(0.0) as u64)
            .context("metricData has no last")?["average"]
            .as_f64()
            .context("metricData last average no exist")?;
        Ok(data)
    }

    /// Query the CPU usage percentage of a particular server.
    async fn cpu_usage_percent(&self, aws_name: &str) -> anyhow::Result<f64> {
        let LightsailConfig {
            access_key_id,
            secret_access_key,
            region,
            availability_zone: _,
            bundle_id: _,
            key_pair_name: _,
            target_cpu_usage: _,
        } = &self.cfg;
        let result = system(&format!("AWS_ACCESS_KEY_ID={access_key_id} AWS_SECRET_ACCESS_KEY={secret_access_key} AWS_DEFAULT_REGION={region} aws lightsail get-instance-metric-data --instance-name {aws_name} --metric-name CPUUtilization --unit Percent --start-time $(date -u -d '1 hour ago' +%FT%TZ) --end-time $(date -u +%FT%TZ) --period 600 --statistics Average")).await?;
        let result: serde_json::Value = serde_json::from_str(&result)?;
        let data = result["metricData"]
            .as_array()
            .context("metricData not an array")?
            .iter()
            .max_by_key(|v| v["timestamp"].as_f64().unwrap_or(0.0) as u64)
            .context("metricData has no last")?["average"]
            .as_f64()
            .context("metricData last average no exist")?;
        Ok(data)
    }
}

#[async_trait]
impl Provider for LightsailProvider {
    async fn create_server(&self, id: &str) -> anyhow::Result<String> {
        let name = id_to_name(id);
        let bundle_id = self.cfg.bundle_id.clone();
        let availability_zone = self.cfg.availability_zone.clone();
        let key_pair_name = self.cfg.key_pair_name.clone();
        let access_key_id = self.cfg.access_key_id.clone();
        let secret_access_key = self.cfg.secret_access_key.clone();
        let region = self.cfg.region.clone();
        system(&format!("AWS_ACCESS_KEY_ID={access_key_id} AWS_SECRET_ACCESS_KEY={secret_access_key} AWS_DEFAULT_REGION={region} aws lightsail create-instances --instance-names {name} --blueprint-id debian_11 --bundle-id {bundle_id} --availability-zone {availability_zone} --key-pair-name {key_pair_name}")).await?;
        log::debug!("<{availability_zone}> created a lightsail instance {name} in");
        let ip_addr = loop {
            let fallible_part = async {
                let s = system(&format!("AWS_ACCESS_KEY_ID={access_key_id} AWS_SECRET_ACCESS_KEY={secret_access_key} AWS_DEFAULT_REGION={region} aws lightsail get-instance --instance-name {name}")).await?;
                let j: SingleInstance = serde_json::from_str(&s)?;
                if let Some(j) = j.instance.public_ip_address {
                    anyhow::Ok(j)
                } else {
                    anyhow::bail!("no IP yet!")
                }
            };
            match fallible_part.await {
                Ok(res) => break res,
                Err(err) => {
                    log::debug!("no IP ({:?}), waiting...", err);
                }
            }
            smol::Timer::after(Duration::from_secs(1)).await;
        };
        wait_until_reachable(&ip_addr).await;
        log::debug!("<{availability_zone}> instance {name} opening ports");
        while let Err(err) = system(&format!(r#"AWS_ACCESS_KEY_ID={access_key_id} AWS_SECRET_ACCESS_KEY={secret_access_key} AWS_DEFAULT_REGION={region} aws lightsail open-instance-public-ports --instance-name {name} --port-info '{{"fromPort": 0, "toPort": 65535, "protocol": "all", "cidrs": ["0.0.0.0/0"]}}'"#)).await {
                log::warn!("retrying... {:?}", err);
                smol::Timer::after(Duration::from_secs(10)).await;
            }
        log::debug!(
            "<{availability_zone}> instance {name} has ip {ip_addr}, enabling root access..."
        );
        system(&format!("ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null admin@{ip_addr} sudo cp ~admin/.ssh/authorized_keys ~root/.ssh/authorized_keys")).await?;
        Ok(ip_addr)
    }

    async fn retain_by_id(
        &self,
        pred: Box<dyn Fn(String) -> bool + Send + 'static>,
    ) -> anyhow::Result<()> {
        let availability_zone = self.cfg.availability_zone.clone(); // TODO take into account!!
        let access_key_id = self.cfg.access_key_id.clone();
        let secret_access_key = self.cfg.secret_access_key.clone();
        let region = self.cfg.region.clone();
        let s = system(&format!("AWS_ACCESS_KEY_ID={access_key_id} AWS_SECRET_ACCESS_KEY={secret_access_key} AWS_DEFAULT_REGION={region} aws lightsail get-instances")).await?;
        log::debug!("{} calling retain on aws", availability_zone);
        let j: MultiInstances = serde_json::from_str(&s)?;
        for instance in j.instances {
            // if instance.state["name"] != "running" {
            //     log::debug!(
            //         "skipping instance {} when delete due to not running",
            //         instance.name
            //     );
            //     continue;
            // }
            if !pred(instance.name.replace("aws-phalanx-", ""))
                && instance.name.contains("aws-phalanx-")
            {
                let availability_zone = availability_zone.clone();
                let instance = instance.clone();
                let access_key_id = access_key_id.clone();
                let secret_access_key = secret_access_key.clone();
                let region = region.clone();
                let instance_name = instance.name;
                if let Err(err) = system(&format!("AWS_ACCESS_KEY_ID={access_key_id} AWS_SECRET_ACCESS_KEY={secret_access_key} AWS_DEFAULT_REGION={region} aws lightsail delete-instance --instance-name {instance_name}")).await {
                    log::warn!(
                        "<{availability_zone}> FAILED TO delete {} {:?}: {:?}",
                        instance_name,
                        instance.public_ip_address,
                        err
                    );
                } else {
                log::warn!(
                    "<{availability_zone}> deleted {} {:?}",
                    instance_name,
                    instance.public_ip_address
                );
            }
            }
        }
        Ok(())
    }

    async fn overload(&self) -> anyhow::Result<f64> {
        let LightsailConfig {
            access_key_id,
            secret_access_key,
            region,
            availability_zone: _,
            bundle_id: _,
            key_pair_name: _,
            target_cpu_usage,
        } = &self.cfg;
        let s = system(&format!("AWS_ACCESS_KEY_ID={access_key_id} AWS_SECRET_ACCESS_KEY={secret_access_key} AWS_DEFAULT_REGION={region} aws lightsail get-instances")).await?;
        let j: MultiInstances = serde_json::from_str(&s)?;

        let mut cpu_usages = futures_util::stream::iter(j.instances.clone())
            .map(|instance| async move {
                (
                    instance.name.clone(),
                    self.cpu_usage_percent(&instance.name).await.ok(),
                )
            })
            .buffer_unordered(6);

        let mut total_cpu_usage = 0.0;
        let mut count = 0;
        while let Some((instance_name, cpu)) = cpu_usages.next().await {
            if let Some(cpu) = cpu {
                total_cpu_usage += cpu;
                log::debug!("{instance_name} has CPU usage {:.2}%", cpu);
                count += 1;
            }
        }

        Ok((total_cpu_usage / count as f64) / *target_cpu_usage)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SingleInstance {
    instance: Inner,
}

#[derive(Debug, Clone, Deserialize)]
struct MultiInstances {
    instances: Vec<Inner>,
}

#[derive(Debug, Clone, Deserialize)]
struct Inner {
    name: String,
    #[serde(rename = "publicIpAddress")]
    public_ip_address: Option<String>,

    state: serde_json::Value,
}

/// mangle a bridge ID to an AWS name
fn id_to_name(id: &str) -> String {
    format!("aws-phalanx-{}", id)
}
