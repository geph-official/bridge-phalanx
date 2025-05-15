pub mod hetzner;
pub mod ip_fresher;
pub mod lightsail;
pub mod linode;
pub mod oneprovider;
pub mod ovh;
pub mod scaleway;
pub mod serverspace;
pub mod vultr;

use std::{process::Stdio, time::Duration};

use async_trait::async_trait;

/// A specific service provider.
#[async_trait]
pub trait Provider: Send + Sync + 'static {
    /// Creates a new server, returning an IP address reachable through SSH port 22 and "root".
    async fn create_server(&self, id: &str) -> anyhow::Result<String>;

    /// Retains only the servers that match the given predicate.
    async fn retain_by_id(
        &self,
        pred: Box<dyn Fn(String) -> bool + Send + 'static>,
    ) -> anyhow::Result<()>;
}

async fn system(cmd: &str) -> anyhow::Result<String> {
    // static SEMAPH: Semaphore = Semaphore::new(16);
    // let _guard = SEMAPH.acquire().await;
    let child = smol::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let output = child.output().await?;
    let std_output: String = String::from_utf8_lossy(&output.stdout).into();
    let std_err: String = String::from_utf8_lossy(&output.stderr).into();
    if std_err.contains("An error") || !output.status.success() {
        anyhow::bail!("{}", std_err.trim())
    }
    if cmd.contains("ssh") {
        log::debug!("SYSTEM >> {}\n<< {:?}  / {:?}", cmd, std_output, std_err);
    }
    anyhow::Ok(std_output)
}

async fn wait_until_reachable(ip: &str) {
    log::debug!("waiting until {ip} is reachable...");
    while let Err(err) = system(&format!("nc -vzw 2 {ip} 22")).await {
        log::error!("{:?}", err);
        smol::Timer::after(Duration::from_secs(2)).await;
    }
}
