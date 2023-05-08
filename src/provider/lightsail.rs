use std::time::Duration;

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
}

pub struct LightsailProvider {
    cfg: LightsailConfig,
}

impl LightsailProvider {
    /// Creates a new Lightsail provider.
    pub fn new(cfg: LightsailConfig) -> Self {
        Self { cfg }
    }
}

impl Provider for LightsailProvider {
    fn create_server(&self, id: &str) -> smol::Task<anyhow::Result<String>> {
        let name = id_to_name(id);
        let bundle_id = self.cfg.bundle_id.clone();
        let availability_zone = self.cfg.availability_zone.clone();
        let key_pair_name = self.cfg.key_pair_name.clone();
        let access_key_id = self.cfg.access_key_id.clone();
        let secret_access_key = self.cfg.secret_access_key.clone();
        let region = self.cfg.region.clone();
        smol::spawn(async move {
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
            system(&format!("ssh -o StrictHostKeyChecking=no admin@{ip_addr} sudo cp ~admin/.ssh/authorized_keys ~root/.ssh/authorized_keys")).await?;
            Ok(ip_addr)
        })
    }

    fn retain_by_id(
        &self,
        pred: Box<dyn Fn(&str) -> bool + Send + 'static>,
    ) -> smol::Task<anyhow::Result<()>> {
        let availability_zone = self.cfg.availability_zone.clone(); // TODO take into account!!
        let access_key_id = self.cfg.access_key_id.clone();
        let secret_access_key = self.cfg.secret_access_key.clone();
        let region = self.cfg.region.clone();
        smol::spawn(async move {
            let s = system(&format!("AWS_ACCESS_KEY_ID={access_key_id} AWS_SECRET_ACCESS_KEY={secret_access_key} AWS_DEFAULT_REGION={region} aws lightsail get-instances")).await?;
            let j: MultiInstances = serde_json::from_str(&s)?;
            for instance in j.instances {
                if !pred(&instance.name.replace("aws-phalanx-", ""))
                    && instance.name.contains("aws-phalanx-")
                {
                    let availability_zone = availability_zone.clone();
                    let instance = instance.clone();
                    let access_key_id = access_key_id.clone();
                    let secret_access_key = secret_access_key.clone();
                    let region = region.clone();
                    let instance_name = instance.name;
                    system(&format!("AWS_ACCESS_KEY_ID={access_key_id} AWS_SECRET_ACCESS_KEY={secret_access_key} AWS_DEFAULT_REGION={region} aws lightsail delete-instance --instance-name {instance_name}")).await?;
                    log::warn!(
                        "<{availability_zone}> deleted {} {:?}",
                        instance_name,
                        instance.public_ip_address
                    );
                }
            }
            Ok(())
        })
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
}

/// mangle a bridge ID to an AWS name
fn id_to_name(id: &str) -> String {
    format!("aws-phalanx-{}", id)
}
