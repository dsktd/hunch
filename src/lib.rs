//! Hunch: a party trivia game for the big screen, phones as answer pads.
//!
//! The rules, questions and phone protocol here are engine-free and tested without a GPU; the TV
//! screens build on them.

pub mod filter;
pub mod fuzzy;
pub mod game;
pub mod phone;
mod phone_guard;
pub mod protocol;
pub mod questions;
pub mod score;
