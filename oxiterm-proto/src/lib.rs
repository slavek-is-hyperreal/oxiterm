//! Protocol and core data types for the OxiTerm framework.
//!
//! This module defines the DOM tree structures, the ANSI-based styling system,
//! and representations for terminal input events (keyboard, mouse).

#![allow(
    clippy::doc_markdown,
    clippy::struct_excessive_bools,
    clippy::too_many_lines
)]

pub mod dom;
pub mod style;
pub mod input;

pub use dom::{Node, NodeId, NodeTag, NodeAttributes};
pub use style::{AnsiColor, ComputedStyle, BorderStyle, BorderChars, ColorDepth, TerminalProfile};
pub use input::{InputEvent, KeyEvent, KeyKind, KeyModifiers, MouseInput, MouseButton, MouseAction};
