use std::process::Stdio;

use once_cell::sync::Lazy;
use smol::lock::Semaphore;

pub async fn ssh_execute(host: &str, cmd: &str) -> anyhow::Result<String> {
    static SSH_SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(64));
    let _guard = SSH_SEMAPHORE.acquire().await;

    let status = smol::process::Command::new("ssh")
        .arg("-o")
        .arg("StrictHostKeyChecking=no")
        .arg(format!("root@{host}"))
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?
        .output()
        .await?;
    if !status.status.success() {
        anyhow::bail!("failed with status {:?}", status)
    }
    Ok(String::from_utf8_lossy(&status.stdout).into())
}
