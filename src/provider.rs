pub mod lightsail;

use smol::Task;

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
