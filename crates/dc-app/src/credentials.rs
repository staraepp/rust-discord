//! OS credential access. Never prints or persists a plaintext fallback.
use anyhow::{Result, bail};
use dc_protocol::Secret;
const SERVICE: &str = "dc-client";
const ACCOUNT: &str = "discord-session";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|_| anyhow::anyhow!("Could not initialize the OS credential store"))
}
fn on_keyring_thread<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    std::thread::spawn(f)
        .join()
        .map_err(|_| anyhow::anyhow!("OS credential worker failed"))?
}
pub fn load() -> Result<Secret<String>> {
    on_keyring_thread(|| {
        let value=entry()?.get_password().map_err(|_|anyhow::anyhow!("No usable OS credential. Set DC_TOKEN for this session, or use dc-client --store-token with DC_TOKEN set."))?;
        if value.trim().is_empty() {
            bail!("The stored credential is empty");
        }
        Ok(Secret::new(value))
    })
}
pub fn save(token: Secret<String>) -> Result<()> {
    on_keyring_thread(move || {
        entry()?.set_password(token.expose_secret()).map_err(|_| {
            anyhow::anyhow!(
                "Could not save to the OS credential store; no plaintext fallback was written"
            )
        })
    })
}
pub fn forget() -> Result<()> {
    on_keyring_thread(|| match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => bail!("Could not remove the OS credential"),
    })
}
