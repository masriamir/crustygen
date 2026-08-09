//! Generates playable Doom maps from a room-graph intermediate representation.
//!
//! The compiler turns a hand-authored IR into UDMF `TEXTMAP` text, enforcing
//! geometry integrity and playability structurally rather than by inspection.

pub mod geom;
pub mod ir;
pub mod tables;
