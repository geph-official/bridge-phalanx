use anyhow::Result;
use async_trait::async_trait;
use sqlx::query;

use super::Provider;
use crate::database::DATABASE;

pub struct IpFresher<T: Provider> {
    inner: T,
}

impl<T: Provider> IpFresher<T> {
    pub fn new(provider: T) -> Self {
        Self { inner: provider }
    }

    // Helper function to check if an IP has been seen before
    async fn is_ip_seen(&self, ip: &str) -> Result<bool> {
        let result: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM phalanx_seen_ips WHERE ip_address = $1)",
        )
        .bind(ip)
        .fetch_one(&*DATABASE)
        .await?;

        Ok(result)
    }

    // Helper function to record a seen IP
    async fn record_seen_ip(&self, ip: &str) -> Result<()> {
        sqlx::query("INSERT INTO phalanx_seen_ips (ip_address) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(ip)
            .execute(&*DATABASE)
            .await?;

        Ok(())
    }
}

#[async_trait]
impl<T: Provider> Provider for IpFresher<T> {
    async fn create_server(&self, id: &str) -> Result<String> {
        loop {
            let ip = self.inner.create_server(id).await?;

            // Check if we've seen this IP before
            let seen = self.is_ip_seen(&ip).await?;

            if !seen {
                // If this IP hasn't been seen before, add it to our database and return it
                self.record_seen_ip(&ip).await?;
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
