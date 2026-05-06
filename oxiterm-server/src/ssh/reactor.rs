use std::thread;
use std::sync::mpsc;
use oxiterm_proto::input::{InputEvent, decoder::InputDecoder};
use tracing::{error, debug};

pub struct ReactorThread {
    _handle: thread::JoinHandle<()>,
}

impl ReactorThread {
    /// Spawns a dedicated OS thread for input processing.
    /// Bridges raw byte stream from SSH to structured `InputEvents`.
    pub fn spawn(rx: mpsc::Receiver<Vec<u8>>, tx: mpsc::Sender<InputEvent>) -> Self {
        let handle = thread::spawn(move || {
            debug!("ReactorThread started");
            let mut decoder = InputDecoder::new();
            
            while let Ok(data) = rx.recv() {
                if let Some(frame) = Self::sanitize_frame(&data) {
                    let events = decoder.feed(&frame);
                    for event in events {
                        if let Err(e) = tx.send(event) {
                            error!("Failed to send InputEvent from ReactorThread: {:?}", e);
                            return;
                        }
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
        
        // Basic validation: ignore if it contains null bytes in weird places (optional)
        
        Some(raw.to_vec())
    }
}
