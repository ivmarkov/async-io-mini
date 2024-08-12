//! Async I/O for the ESP IDF (and possibly other MCU RTOSes supporting the [select] call and BSD Sockets).
//!
//! This crate provides [`Async`], an adapter for standard networking types (and [many other] types) to use in
//! async programs.
//!
//! # Implementation
//!
//! The first time [`Async`] is used, a thread called "async-io-mini" will be spawned.
//! The purpose of this thread is to wait for I/O events reported by the OS, and then
//! wake appropriate futures blocked on I/O when they can be resumed.
//!
//! To wait for the next I/O event, the task uses the [select] syscall available on many operating systems.
//!
//! # Examples
//!
//! Connect to `example.com:80`.
//!
//! ```
//! use async_io_mini::Async;
//!
//! use std::net::{TcpStream, ToSocketAddrs};
//!
//! # futures_lite::future::block_on(async {
//! let addr = "example.com:80".to_socket_addrs()?.next().unwrap();
//!
//! let stream = Async::<TcpStream>::connect(addr).await?;
//! # std::io::Result::Ok(()) });
//! ```

#![allow(unknown_lints)]
#![allow(clippy::needless_maybe_sized)]

pub use io::*;
#[cfg(feature = "embassy-time")]
pub use timer::*;

mod io;
mod reactor;
mod sys;
#[cfg(feature = "embassy-time")]
mod timer;
