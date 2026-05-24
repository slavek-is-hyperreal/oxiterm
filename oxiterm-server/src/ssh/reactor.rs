use std::thread;
use std::sync::mpsc;
use oxiterm_proto::input::{InputEvent, decoder::InputStateMachine};
use tracing::{error, debug};

pub enum ReactorMessage {
    Raw(Vec<u8>),
    Resize(u16, u16),
}

pub struct ReactorThread {
    _handle: thread::JoinHandle<()>,
}

impl ReactorThread {
    /// Spawns a dedicated OS thread for input processing.
    /// Bridges raw byte stream from SSH to structured `InputEvents`.
    pub fn spawn(rx: mpsc::Receiver<ReactorMessage>, tx: crate::backpressure::BoundedFrameChannel<InputEvent>) -> Self {
        let handle = thread::spawn(move || {
            debug!("ReactorThread started");
            let mut decoder = InputStateMachine::new();
            
            while let Ok(msg) = rx.recv() {
                match msg {
                    ReactorMessage::Raw(data) => {
                        if let Some(frame) = Self::sanitize_frame(&data) {
                            let events = decoder.feed_slice(&frame);
                            for event in events {
                                if tx.try_send(event) == crate::backpressure::SendResult::Closed {
                                    error!("Failed to send InputEvent from ReactorThread: Channel Closed");
                                    return;
                                }
                            }
                        }
                    }
                    ReactorMessage::Resize(cols, rows) => {
                        let _ = tx.try_send(InputEvent::Resize { cols, rows });
                    }
                }
            }
            debug!("ReactorThread exiting");
        });

        Self { _handle: handle }
    }

    /// S4-04: Sanitization of raw frames to prevent malformed sequence attacks.
    fn sanitize_frame(raw: &[u8]) -> Option<Vec<u8>> {
        // Limit frame size to prevent DoS via huge escape sequences
        if raw.len() > 4096 {
            debug!("Dropped oversized frame: {} bytes", raw.len());
            return None;
        }
        
        Some(raw.to_vec())
    }
}
