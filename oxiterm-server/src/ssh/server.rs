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
    pub rate_limiter: Arc<crate::ratelimit::RateLimiter>,
    pub peer_addr: std::net::SocketAddr,
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
        info!("Opening session on channel: {}", channel.id());
        
        // BUG-H06: Rate limiting
        let ip = self.peer_addr.ip();
        match self.rate_limiter.check_and_record(ip) {
            crate::ratelimit::RateResult::Deny => {
                warn!("Rate limit exceeded for IP: {ip}, denying session");
                return Ok(false);
            }
            crate::ratelimit::RateResult::Throttle(delay) => {
                info!("Throttling session for IP: {ip} for {:?}", delay);
                tokio::time::sleep(delay).await;
            }
            crate::ratelimit::RateResult::Allow => {}
        }

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

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        info!("Shell request on channel: {channel:?}");
        crate::ssh::negotiator::negotiate_capabilities(channel, session)?;
        
        let sid = self.channels.lock().get(&channel).copied();
        if let Some(sid) = sid {
            if let Some(client_session) = self.registry.sessions.read().get(&sid).cloned() {
                let handle = session.handle();
                let (output_tx, mut output_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
                
                let event_bus = Arc::new(crate::events::EventBus::new());
                let mut event_loop = crate::session::EventLoop::new(client_session, event_bus, output_tx);
                
                std::thread::spawn(move || {
                    event_loop.run();
                });
                
                tokio::spawn(async move {
                    while let Some(data) = output_rx.recv().await {
                        let _ = handle.data(channel, data.into()).await;
                    }
                });
            }
        }
        Ok(())
    }

    async fn exec_request(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        warn!("Blocking exec request on channel {channel:?}: {}", String::from_utf8_lossy(data));
        session.request_failure();
        Ok(()) 
    }

    async fn subsystem_request(&mut self, channel: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        warn!("Blocking subsystem request on channel {channel:?}: {name}");
        session.request_failure();
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
                let new_dims = crate::session::PtyDimensions {
                    cols: u16::try_from(width).unwrap_or(u16::MAX),
                    rows: u16::try_from(height).unwrap_or(u16::MAX),
                };
                *session.dims.write() = new_dims;
                session.resize_debouncer.write().push(new_dims);
                // BUG-H03: Use ReactorMessage::Resize instead of empty Vec
                let _ = session.raw_input_tx.send(crate::ssh::reactor::ReactorMessage::Resize(new_dims.cols, new_dims.rows));
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

    async fn data(&mut self, channel: ChannelId, data: &[u8], _session: &mut Session) -> Result<(), Self::Error> {
        let sid = self.channels.lock().get(&channel).copied();
        if let Some(sid) = sid {
            if let Some(session) = self.registry.sessions.read().get(&sid) {
                // Send raw data to RRT
                if let Err(e) = session.raw_input_tx.send(crate::ssh::reactor::ReactorMessage::Raw(data.to_vec())) {
                    warn!("Failed to send data to reactor for session {sid}: {e:?}");
                }
            }
        }
        Ok(())
    }
}
