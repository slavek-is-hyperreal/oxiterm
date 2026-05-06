use std::sync::Arc;
use async_trait::async_trait;
use russh::{server, server::{Session, Handler}, ChannelId, Channel};
use russh_keys::key;
use tracing::info;
use crate::session::{SessionRegistry, SessionId};

#[derive(Clone)]
pub struct OxiServer {
    pub registry: Arc<SessionRegistry>,
    pub current_session: Option<SessionId>,
}

#[async_trait]
impl Handler for OxiServer {
    type Error = anyhow::Error;

    async fn auth_publickey(&mut self, user: &str, public_key: &key::PublicKey) -> Result<server::Auth, Self::Error> {
        info!("Auth attempt for user: {} with key: {:?}", user, public_key);
        // TODO: implement actual key verification (S1-07)
        Ok(server::Auth::Accept)
    }

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Reject { proceed_with_methods: None })
    }

    async fn channel_open_session(&mut self, channel: Channel<russh::server::Msg>, _session: &mut Session) -> Result<bool, Self::Error> {
        info!("Opening session on channel: {:?}", channel.id());
        let client_session = self.registry.create_session();
        self.current_session = Some(client_session.id);
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        _channel: ChannelId,
        term: &str,
        width: u32,
        height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        info!("PTY request: {}x{} (term: {})", width, height, term);
        if let Some(sid) = self.current_session {
            if let Some(sessions) = self.registry.sessions.read().get(&sid) {
                *sessions.dims.write() = crate::session::PtyDimensions {
                    cols: width as u16,
                    rows: height as u16,
                };
            }
        }
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        info!("Shell request on channel: {:?}", channel);
        // Start SSR engine here (Sprint 4+)
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        width: u32,
        height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        info!("Window change request: {}x{}", width, height);
        if let Some(sid) = self.current_session {
            if let Some(sessions) = self.registry.sessions.read().get(&sid) {
                *sessions.dims.write() = crate::session::PtyDimensions {
                    cols: width as u16,
                    rows: height as u16,
                };
            }
        }
        Ok(())
    }
}
