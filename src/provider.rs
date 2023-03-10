pub mod lightsail;
pub mod vultr;

use std::process::Stdio;

use smol::{lock::Semaphore, Task};

/// A specific service provider.
pub trait Provider: Send + Sync + 'static {
    /// Creates a new server, returning an IP address reachable through SSH port 22 and "root".
    fn create_server(&self, id: &str) -> Task<anyhow::Result<String>>;

    /// Retains only the servers that match the given predicate.
    fn retain_by_id(
        &self,
        pred: Box<dyn Fn(&str) -> bool + Send + 'static>,
    ) -> Task<anyhow::Result<()>>;
}

async fn system(cmd: &str) -> anyhow::Result<String> {
    static SEMAPH: Semaphore = Semaphore::new(6);
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
