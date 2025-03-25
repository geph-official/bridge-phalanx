use std::{sync::LazyLock, time::Duration};

use async_trait::async_trait;
use dashmap::DashSet;
use isahc::AsyncReadResponseExt;
use serde::{Deserialize, Serialize};

use super::{wait_until_reachable, Provider};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LinodeConfig {
    pub api_token: String,
    pub region: String,
    pub type_id: String,
    pub image: String,
    pub root_pass: String,
    pub authorized_keys: Vec<String>,
}

pub struct LinodeProvider {
    client: isahc::HttpClient,
    cfg: LinodeConfig,
}

impl LinodeProvider {
    /// Create a new Linode-based provider.
    pub fn new(cfg: LinodeConfig) -> Self {
        let client = isahc::HttpClientBuilder::new()
            .default_headers(&[
                (
                    "Authorization",
                    format!("Bearer {}", cfg.api_token).as_str(),
                ),
                ("Content-Type", "application/json"),
            ])
            .build()
            .unwrap();
        Self { client, cfg }
    }
}

#[derive(Clone, Debug, Serialize)]
struct CreateLinodeArgs {
    region: String,
    r#type: String,
    image: String,
    label: String,
    root_pass: String,
    authorized_keys: Vec<String>,
    booted: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct LinodeInstance {
    id: i32,
    label: String,
    status: String,
    ipv4: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct LinodeListResponse {
    data: Vec<LinodeInstance>,
}

fn linodify_id(id: &str) -> String {
    format!("bridge-{id}")
}

static CREATING: LazyLock<DashSet<i32>> = LazyLock::new(DashSet::new);

#[async_trait]
impl Provider for LinodeProvider {
    async fn create_server(&self, id: &str) -> anyhow::Result<String> {
        let id = id.to_string();
        let cfg = self.cfg.clone();
        let client = self.client.clone();
        let req = CreateLinodeArgs {
            label: linodify_id(&id),
            region: cfg.region.clone(),
            r#type: cfg.type_id.clone(),
            image: cfg.image.clone(),
            root_pass: cfg.root_pass.clone(),
            authorized_keys: cfg.authorized_keys.clone(),
            booted: true,
        };

        let mut resp = client
            .post_async(
                "https://api.linode.com/v4/linode/instances",
                serde_json::to_vec(&req)?,
            )
            .await?;

        if !resp.status().is_success() {
            let r = resp.text().await?;
            anyhow::bail!("non-success while creating Linode: {:?} {r}", resp.status())
        }

        let linode: LinodeInstance = resp.json().await?;
        CREATING.insert(linode.id);
        let linode_id = linode.id;
        scopeguard::defer!({
            CREATING.remove(&linode_id);
        });
        // Wait for the Linode to be fully provisioned and running
        loop {
            let instance = self.get_server_by_id(&linode.id.to_string()).await?;

            if instance.status == "running" && !instance.ipv4.is_empty() {
                wait_until_reachable(&instance.ipv4[0]).await;
                return Ok(instance.ipv4[0].clone());
            }

            smol::Timer::after(Duration::from_secs(5)).await;
        }
    }

    async fn retain_by_id(
        &self,
        pred: Box<dyn Fn(String) -> bool + Send + 'static>,
    ) -> anyhow::Result<()> {
        let instances = self.list_all().await?;

        for instance in instances {
            let id = instance
                .label
                .strip_prefix("bridge-")
                .unwrap_or(&instance.label)
                .to_string();

            if !pred(id.clone()) && !CREATING.contains(&instance.id) {
                log::debug!("MUST DELETE {id}");
                smol::Timer::after(Duration::from_secs(30)).await;
                self.delete_server(&instance.id.to_string()).await?;
            }
        }

        Ok(())
    }
}

impl LinodeProvider {
    async fn list_all(&self) -> anyhow::Result<Vec<LinodeInstance>> {
        let mut resp = self
            .client
            .get_async("https://api.linode.com/v4/linode/instances")
            .await?;

        if !resp.status().is_success() {
            let r = resp.text().await?;
            anyhow::bail!("non-success while listing Linodes: {:?} {r}", resp.status())
        }

        let response: LinodeListResponse = resp.json().await?;

        // Filter instances to only include those with our naming pattern
        Ok(response
            .data
            .into_iter()
            .filter(|instance| instance.label.starts_with("bridge-"))
            .collect())
    }

    async fn get_server_by_id(&self, id: &str) -> anyhow::Result<LinodeInstance> {
        let mut resp = self
            .client
            .get_async(&format!(
                "https://api.linode.com/v4/linode/instances/{}",
                id
            ))
            .await?;

        if !resp.status().is_success() {
            let r = resp.text().await?;
            anyhow::bail!(
                "non-success while getting Linode details: {:?} {r}",
                resp.status()
            )
        }

        let instance: LinodeInstance = resp.json().await?;
        Ok(instance)
    }

    async fn delete_server(&self, id: &str) -> anyhow::Result<()> {
        let mut resp = self
            .client
            .delete_async(&format!(
                "https://api.linode.com/v4/linode/instances/{}",
                id
            ))
            .await?;

        log::debug!("LINODE DELETING {id}");

        if !resp.status().is_success() {
            let r = resp.text().await?;
            anyhow::bail!("non-success while deleting Linode: {:?} {r}", resp.status())
        }

        Ok(())
    }
}
