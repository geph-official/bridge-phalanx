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
    pub provider: ProviderConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ProviderConfig {
    Lightsail(LightsailConfig),
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

/// Global configuration file
pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    let bts = std::fs::read(&std::env::args().collect::<Vec<_>>()[1]).unwrap();

    serde_yaml::from_slice(&bts).unwrap()
});
