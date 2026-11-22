//! Server-side implementation of the event loop's queue traits on top of the backpressure
//! channel (condvar wait plus tokio `Notify` for async consumers). Host-only: this is the part
//! that does not move into `oxiterm-core`.

use crate::backpressure::{BoundedFrameChannel, Receiver};
use crate::loop_io::{EventPush, EventSource, WaitResult};
use oxiterm_proto::input::InputEvent;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

impl EventSource for Receiver<InputEvent> {
    fn wait_event(&mut self, timeout: Duration) -> WaitResult {
        match Receiver::recv_timeout(self, timeout) {
            Ok(e) => WaitResult::Event(e),
            Err(RecvTimeoutError::Timeout) => WaitResult::Timeout,
            Err(RecvTimeoutError::Disconnected) => WaitResult::Disconnected,
        }
    }
    fn try_recv(&mut self) -> Option<InputEvent> {
        Receiver::try_recv(self)
    }
}

impl EventPush for BoundedFrameChannel<InputEvent> {
    fn push(&self, event: InputEvent) {
        let _ = self.try_send(event);
    }
}
