//! Adapted from MonaLauncher (GPLv3). Secrets and authorization stay on the host.
pub mod channel;
pub mod chat;
#[cfg(test)]
mod native_tests;
pub mod protocol;
pub mod service;
#[cfg(windows)]
mod windows_channel;

impl service::SessionSource for super::Session {
    fn valid(&self) -> bool {
        self.is_current()
    }
    fn current(&self) -> Result<super::Session, protocol::BrokerError> {
        if self.is_current() {
            Ok(self.clone())
        } else {
            Err(protocol::BrokerError::Revoked)
        }
    }
}
