use std::sync::Arc;
use std::collections::HashMap;
use std::path::PathBuf;
use async_trait::async_trait;
use russh::{server, server::{Session, Handler}, ChannelId, Channel};
use russh_keys::key;
use tracing::{info, warn};
use crate::session::{SessionRegistry, SessionId, THTMLDocument};
use crate::ssh::keys::AuthorizedKeys;

#[derive(Clone)]
pub struct OxiServer {
    pub config: crate::config::OxiTermConfig,
    pub registry: Arc<SessionRegistry>,
    pub auth_keys: Arc<AuthorizedKeys>,
    pub rate_limiter: Arc<crate::ratelimit::RateLimiter>,
    pub peer_addr: std::net::SocketAddr,
    /// Map of SSH channels to `OxiTerm` session IDs
    pub channels: Arc<parking_lot::Mutex<HashMap<ChannelId, SessionId>>>,
    pub initial_document: Option<THTMLDocument>,
    pub source_path: Option<PathBuf>,
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

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<russh::server::Auth, Self::Error> {
        if self.config.server.no_auth {
            warn!("╔══════════════════════════════════════════════════╗");
            warn!("║  ⚠️  NO-AUTH MODE: accepting without credentials  ║");
            warn!("║  NEVER USE THIS IN PRODUCTION ENVIRONMENTS!      ║");
            warn!("╚══════════════════════════════════════════════════╝");
            return Ok(russh::server::Auth::Accept);
        }

        if let Some(ref required_password) = self.config.server.password {
            use subtle::ConstantTimeEq;
            let password_bytes = password.as_bytes();
            let required_bytes = required_password.as_bytes();
            let len_match = password_bytes.len() == required_bytes.len();
            let (to_compare_a, to_compare_b) = if len_match {
                (password_bytes, required_bytes)
            } else {
                (required_bytes, required_bytes)
            };
            let match_bytes = to_compare_a.ct_eq(to_compare_b).unwrap_u8() == 1;
            if !password.is_empty() && len_match && match_bytes {
                return Ok(russh::server::Auth::Accept);
            }
        }
        
        warn!("Auth rejected for {} (unauthorized or missing password)", self.peer_addr);
        Ok(russh::server::Auth::Reject { proceed_with_methods: None })
    }

    async fn channel_open_session(&mut self, channel: Channel<russh::server::Msg>, _session: &mut Session) -> Result<bool, Self::Error> {
        info!("Opening session on channel: {}", channel.id());
        
        // BUG-H06: Rate limiting
        // BUG-RATELIMIT-01: Removed double check here. 
        // Rate limit is already checked at the TCP accept level in mod.rs.
        if let Some(client_session) = self.registry.create_session() {
            self.channels.lock().insert(channel.id(), client_session.id);
            Ok(true)
        } else {
            warn!("Rejecting SSH channel: session registry full");
            Ok(false)
        }
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
                let new_dims = crate::session::PtyDimensions {
                    cols: u16::try_from(width).unwrap_or(u16::MAX),
                    rows: u16::try_from(height).unwrap_or(u16::MAX),
                };
                *session.dims.write() = new_dims;
                session.resize_debouncer.write().push(new_dims);
                let _ = session.raw_input_tx.send(crate::ssh::reactor::ReactorMessage::Resize(new_dims.cols, new_dims.rows));
            }
        }
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        info!("Shell request on channel: {channel:?}");
        crate::ssh::negotiator::negotiate_capabilities(channel, session)?;
        
        let sid = self.channels.lock().get(&channel).copied();
        if let Some(sid) = sid {
            let client_session = self.registry.sessions.read().get(&sid).cloned();
            if let Some(client_session) = client_session {
                let handle = session.handle();
                let (output_tx, mut output_rx) = crate::backpressure::BoundedFrameChannel::<Vec<u8>>::new(32);
                
                let event_bus = Arc::new(crate::events::EventBus::new());
                let dims = *client_session.dims.read();
                let doc = if let Some(ref path) = self.source_path {
                    match crate::loader::load_thtml_file(path) {
                        Ok(d) => d,
                        Err(e) => {
                            warn!("Failed to load document from source path {:?}: {}, falling back to initial_document", path, e);
                            if let Some(ref initial) = self.initial_document {
                                initial.clone()
                            } else {
                                crate::placeholder::build_placeholder_doc(dims.cols, dims.rows)
                            }
                        }
                    }
                } else if let Some(ref initial) = self.initial_document {
                    initial.clone()
                } else {
                    crate::placeholder::build_placeholder_doc(dims.cols, dims.rows)
                };

                let mut event_loop = crate::session::EventLoop::new(client_session, event_bus, output_tx, doc, self.config.server.a11y_mode);
                event_loop.source_path = self.source_path.clone();
                if let Some(ref url) = self.config.server.app_server_url {
                    event_loop.app_dispatcher = Some(crate::dispatcher::AppDispatcher::new(url.clone()));
                }

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
        info!("SSH Data received on channel {:?}: len={}", channel, data.len());
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
