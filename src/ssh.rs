use std::{process::Stdio, time::Duration};

use anyhow::Context;
use once_cell::sync::Lazy;
use smol::lock::Semaphore;
use smol_timeout::TimeoutExt;

pub async fn ssh_execute(host: &str, cmd: &str) -> anyhow::Result<String> {
    static SSH_SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(512));
    let _guard = SSH_SEMAPHORE.acquire().await;

    let status = smol::process::Command::new("ssh")
        .arg("-C")
        .arg("-o")
        .arg("ConnectTimeout=300")
        .arg("-o")
        .arg("StrictHostKeyChecking=no")
        .arg("-o")
        .arg("UserKnownHostsFile=/dev/null")
        .arg(format!("root@{host}"))
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .output()
        .timeout(Duration::from_secs(300))
        .await
        .context("timeout in SSH after 300 secs")??;

    if !status.status.success() {
        anyhow::bail!("failed with status {:?}", status)
    }
    log::trace!("ssh <{host}> {cmd}");
    Ok(String::from_utf8_lossy(&status.stdout).into())
}
