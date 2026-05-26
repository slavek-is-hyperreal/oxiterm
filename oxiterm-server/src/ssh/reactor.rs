//! Thread reactor for input processing.
//!
//! Spawns a dedicated background thread to parse incoming SSH terminal raw escape sequences
//! and window resize directives into structured input events.

use std::thread;
use std::sync::mpsc;
use oxiterm_proto::input::{InputEvent, decoder::InputStateMachine};
use tracing::{error, debug};

/// Message type sent to the reactor thread.
pub enum ReactorMessage {
    /// Raw byte sequence read from client's interactive channel.
    Raw(Vec<u8>),
    /// Window resize action specifying new dimensions (columns, rows).
    Resize(u16, u16),
}

/// OS thread running the input decoder state machine.
pub struct ReactorThread {
    _handle: thread::JoinHandle<()>,
}

impl ReactorThread {
    /// Spawns a dedicated OS thread for input processing.
    /// Bridges raw byte streams from SSH channels to structured `InputEvent` streams.
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

    /// Sanitizes raw input frames to prevent malformed sequence or buffer overflow attacks.
    ///
    /// Anchored by spec [S4-04]. Discards raw frames exceeding the safety threshold size.
    fn sanitize_frame(raw: &[u8]) -> Option<Vec<u8>> {
        if raw.len() > 4096 {
            debug!("Dropped oversized frame: {} bytes", raw.len());
            return None;
        }
        
        Some(raw.to_vec())
    }
}
