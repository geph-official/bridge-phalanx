use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::Provider;

pub struct IpFresher<T: Provider> {
    inner: T,
    seen_ips: Arc<Mutex<HashSet<String>>>,
}

impl<T: Provider> IpFresher<T> {
    pub fn new(provider: T) -> Self {
        Self {
            inner: provider,
            seen_ips: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn add_seen_ip(&self, ip: String) {
        if let Ok(mut seen_ips) = self.seen_ips.lock() {
            seen_ips.insert(ip);
        }
    }
}

#[async_trait]
impl<T: Provider> Provider for IpFresher<T> {
    async fn create_server(&self, id: &str) -> Result<String> {
        loop {
            let ip = self.inner.create_server(id).await?;

            // Check if we've seen this IP before
            let mut seen = false;
            if let Ok(seen_ips) = self.seen_ips.lock() {
                seen = seen_ips.contains(&ip);
            }

            if !seen {
                // If this IP hasn't been seen before, add it to our set and return it
                self.add_seen_ip(ip.clone());
                return Ok(ip);
            }

            // If we've seen this IP before, try again by continuing the loop
            log::info!("IP {} already seen, retrying server creation", ip);
        }
    }

    async fn retain_by_id(&self, pred: Box<dyn Fn(String) -> bool + Send + 'static>) -> Result<()> {
        // Delegate to the inner provider
        self.inner.retain_by_id(pred).await
    }
}
