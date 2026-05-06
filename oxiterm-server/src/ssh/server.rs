use std::sync::Arc;
use std::collections::HashMap;
use async_trait::async_trait;
use russh::{server, server::{Session, Handler}, ChannelId, Channel};
use russh_keys::key;
use tracing::{info, warn};
use crate::session::{SessionRegistry, SessionId};
use crate::ssh::keys::AuthorizedKeys;

#[derive(Clone)]
pub struct OxiServer {
    pub registry: Arc<SessionRegistry>,
    pub auth_keys: Arc<AuthorizedKeys>,
    /// Map of SSH channels to `OxiTerm` session IDs
    pub channels: Arc<parking_lot::Mutex<HashMap<ChannelId, SessionId>>>,
}

#[async_trait]
impl Handler for OxiServer {
    type Error = anyhow::Error;

    async fn auth_publickey(&mut self, user: &str, public_key: &key::PublicKey) -> Result<server::Auth, Self::Error> {
        info!("Auth attempt for user: {user} with key: {public_key:?}");
        if self.auth_keys.verify(public_key) {
            Ok(server::Auth::Accept)
        } else {
            warn!("Rejected unauthorized key for user: {user}");
            Ok(server::Auth::Reject { proceed_with_methods: None })
        }
    }

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<server::Auth, Self::Error> {
        Ok(server::Auth::Reject { proceed_with_methods: None })
    }

    async fn channel_open_session(&mut self, channel: Channel<russh::server::Msg>, _session: &mut Session) -> Result<bool, Self::Error> {
        info!("Opening session on channel: {:?}", channel.id());
        let client_session = self.registry.create_session();
        self.channels.lock().insert(channel.id(), client_session.id);
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        width: u32,
        height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        info!("PTY request for channel {channel:?}: {width}x{height} (term: {term})");
        let sid = self.channels.lock().get(&channel).copied();
        if let Some(sid) = sid {
            if let Some(session) = self.registry.sessions.read().get(&sid) {
                *session.dims.write() = crate::session::PtyDimensions {
                    cols: u16::try_from(width).unwrap_or(u16::MAX),
                    rows: u16::try_from(height).unwrap_or(u16::MAX),
                };
            }
        }
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, _session: &mut Session) -> Result<(), Self::Error> {
        info!("Shell request on channel: {channel:?}");
        // Start SSR engine here (Sprint 4+)
        Ok(())
    }

    async fn exec_request(&mut self, channel: ChannelId, data: &[u8], _session: &mut Session) -> Result<(), Self::Error> {
        warn!("Blocking exec request on channel {channel:?}: {:?}", String::from_utf8_lossy(data));
        Ok(()) 
    }

    async fn subsystem_request(&mut self, channel: ChannelId, name: &str, _session: &mut Session) -> Result<(), Self::Error> {
        warn!("Blocking subsystem request on channel {channel:?}: {name}");
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        width: u32,
        height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        info!("Window change request for channel {channel:?}: {width}x{height}");
        let sid = self.channels.lock().get(&channel).copied();
        if let Some(sid) = sid {
            if let Some(session) = self.registry.sessions.read().get(&sid) {
                *session.dims.write() = crate::session::PtyDimensions {
                    cols: u16::try_from(width).unwrap_or(u16::MAX),
                    rows: u16::try_from(height).unwrap_or(u16::MAX),
                };
            }
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        info!("Channel {channel:?} closed");
        let sid = self.channels.lock().remove(&channel);
        if let Some(sid) = sid {
            info!("Removing session {sid} from registry");
            self.registry.remove_session(sid);
        }
        Ok(())
    }
}
