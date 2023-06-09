pub mod hetzner;
pub mod lightsail;
pub mod ovh;
pub mod scaleway;
pub mod vultr;

use std::process::Stdio;

use async_trait::async_trait;
use smol::lock::Semaphore;

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

    /// Calculates an "overload" value for all servers that were created by this provider. If this is greater than 1, then more servers may be spawned out of nowhere. Otherwise, the number of servers will be gradually reduced down to the specified value. Not all providers support this method, so it has a default dummy implementation.
    async fn overload(&self) -> anyhow::Result<f64> {
        Ok(0.0)
    }
}

async fn system(cmd: &str) -> anyhow::Result<String> {
    static SEMAPH: Semaphore = Semaphore::new(16);
    let _guard = SEMAPH.acquire().await;
    let child = smol::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let output = child.output().await?;
    let std_output: String = String::from_utf8_lossy(&output.stdout).into();
    let std_err: String = String::from_utf8_lossy(&output.stderr).into();
    if std_err.contains("An error") {
        anyhow::bail!("{}", std_err.trim())
    }
    // eprintln!(">> {}\n<< {}", cmd, output);
    anyhow::Ok(std_output)
}

async fn wait_until_reachable(ip: &str) {
    log::debug!("waiting until {ip} is reachable...");
    while let Err(err) = system(&format!("until nc -vzw 2 {ip} 22; do sleep 2; done")).await {
        log::error!("{:?}", err)
    }
}
