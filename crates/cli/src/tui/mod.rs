//! Full-screen ratatui//! Terminal User Interface for openpista.
#![allow(dead_code, unused_imports)]

pub mod app;
pub mod chat;
pub mod event;
pub mod home;
pub mod status;

pub use event::run_tui;
