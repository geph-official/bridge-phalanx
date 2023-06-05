use std::collections::BTreeMap;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::provider::{
    hetzner::HetznerConfig, lightsail::LightsailConfig, ovh::OVHConfig, scaleway::ScalewayConfig,
    vultr::VultrConfig,
};

#[derive(Serialize, Deserialize, Clone, Debug)]
/// YAML configuration file
pub struct Config {
    /// The URL of the main postgres database.
    pub postgres_url: String,
    /// The bridge secret.
    pub bridge_secret: String,
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
    /// Maximum lifetime.
    pub max_lifetime_hr: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ProviderConfig {
    Lightsail(LightsailConfig),
    Vultr(VultrConfig),
    Scaleway(ScalewayConfig),
    Hetzner(HetznerConfig),
    OVH(OVHConfig),
}

/// Global configuration file
pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    let bts = std::fs::read(&std::env::args().collect::<Vec<_>>()[1]).unwrap();

    serde_yaml::from_slice(&bts).unwrap()
});
