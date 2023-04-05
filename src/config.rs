use std::collections::BTreeMap;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
/// YAML configuration file
pub struct Config {
    /// The URL of the main postgres database.
    pub postgres_url: String,
    /// The bridge secret.
    pub bridge_secret: String,
    /// Maximum lifetime.
    pub max_lifetime_hr: f64,
    /// Bridge groups
    pub groups: BTreeMap<String, GroupConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
/// Configuration for a single bridge group
pub struct GroupConfig {
    pub frontline: usize,
    pub reserve: usize,
    #[serde(default)]
    pub override_group: Option<String>,
    pub provider: ProviderConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ProviderConfig {
    Lightsail(LightsailConfig),
    Vultr(VultrConfig),
    Scaleway(ScalewayConfig),
    ScalewayBaremetal(ScalewayConfig),
    Hetzner(HetznerConfig),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LightsailConfig {
    pub access_key_id: String,
    pub secret_access_key: String,

    pub region: String,
    pub availability_zone: String,

    pub bundle_id: String,
    pub key_pair_name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VultrConfig {
    pub api_key: String,
    pub sshkey_id: String,
    pub region: String,
    pub plan: String,
    pub os_id: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScalewayConfig {
    pub secret_key: String,
    pub zone: String,
    pub project_id: String,
    pub commercial_type: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HetznerConfig {
    pub api_token: String,
    pub server_type: String,
    pub location: String,
    pub image: String,
    pub sshkey_id: String,
}

/// Global configuration file
pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    let bts = std::fs::read(&std::env::args().collect::<Vec<_>>()[1]).unwrap();

    serde_yaml::from_slice(&bts).unwrap()
});
