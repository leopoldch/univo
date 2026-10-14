//! Test helpers shared by the Univo crates. Not published: the crates use it
//! as a development dependency only.

// Helpers panic on a malformed fixture: the test using them fails, as it should.
#![allow(clippy::missing_panics_doc)]

pub mod corpus;
pub mod timing;
