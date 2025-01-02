use std::collections::BTreeMap;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::provider::{
    hetzner::HetznerConfig, lightsail::LightsailConfig, oneprovider::OneCloudConfig,
    ovh::OvhConfig, scaleway::ScalewayConfig, vultr::VultrConfig,
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
    #[serde(default)]
    pub no_antigfw: bool,
    pub provider: ProviderConfig,
    /// Maximum lifetime.
    pub max_lifetime_hr: f64,
    pub services: Vec<Service>,
    pub max_bandwidth_gb: Option<u64>,

    #[serde(default = "huge_mbps")]
    pub target_mbps: f64,
}

fn huge_mbps() -> f64 {
    f64::MAX
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Geph4,
    Geph5,
    Earendil,
}

pub const GEPH4_GIST: &str = "https://gist.githubusercontent.com/nullchinchilla/746ec2007cc293af881f7354405cfb6e/raw/deploy-bridge-geph4.sh";
pub const GEPH5_GIST: &str = "https://gist.githubusercontent.com/nullchinchilla/64a3ded0b62f1decef65c84f43e45dbe/raw/deploy-bridge-geph5.sh";
pub const EARENDIL_GIST: &str = "https://gist.githubusercontent.com/nullchinchilla/26ccd7af71f403df1495e4038a6ce9ff/raw/deploy-bridge-earendil.sh";
pub const LIMIT_BANDWIDTH_GIST: &str = "https://gist.githubusercontent.com/nullchinchilla/4048244030910c0af9b61c42f98d8e65/raw/enforce-bandwidth-max.sh";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ProviderConfig {
    Lightsail(LightsailConfig),
    Vultr(VultrConfig),
    Scaleway(ScalewayConfig),
    Hetzner(HetznerConfig),
    Ovh(OvhConfig),
    Onecloud(OneCloudConfig),
}

/// Global configuration file
pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    let bts = std::fs::read(&std::env::args().collect::<Vec<_>>()[1]).unwrap();

    serde_yaml::from_slice(&bts).unwrap()
});
