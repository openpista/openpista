//! Full-screen ratatui//! Terminal User Interface for openpista.
#![allow(dead_code, unused_imports)]

pub mod action;
pub mod app;
pub mod approval;
pub mod chat;
pub mod event;
pub mod home;
pub mod selection;
pub mod sidebar;
pub mod status;
pub mod theme;

pub use event::run_tui;
