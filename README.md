# async-io-mini

[![CI](https://github.com/ivmarkov/async-io-mini/actions/workflows/ci.yml/badge.svg)](https://github.com/ivmarkov/async-io-mini/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](https://github.com/ivmarkov/async-io-mini)
[![Cargo](https://img.shields.io/crates/v/async-io-mini.svg)](https://crates.io/crates/async-io-mini)
[![Documentation](https://docs.rs/async-io/badge.svg)](https://docs.rs/async-io-mini)

Async I/O. **EXPERIMENTAL!!**

This crate is an **experimental** fork of the splendid [`async-io`](https://github.com/smol-rs/async-io) crate targetting MCUs and ESP-IDF in particular.

## How to use?

`async-io-mini` is a drop-in, API-compatible replacement for the `Async` type from `async-io` (but does NOT have an equivalent of `Timer` - see why [below](#limitations)).

So either:
* Just replace all `use async_io` occurances in your crate with `use async_io_mini`
* Or - in your `Cargo.toml` - replace:
  * `async-io = "..."`
  * with `async-io = { package = "async-io-mini", ... }`

## Justification

While `async-io` supports a ton of operating systems - _including ESP-IDF for the Espressif MCU chips_ - it does have a non-trivial memory consumption in the hidden thread named `async-io`.  Since its hidden `Reactor` object is initialized lazily, it so happens that it is first allocated on-stack, and then it is moved into the static context. This requires the `async-io` thread to have at least 8K stack, which - by MCU standards! - is relatively large if you are memory-constrained.

In contrast, `async-io-mini`:
- Needs < 3K of stack with ESP-IDF (and that's only because ESP-IDF interrupts are executed on the stack of the interrupted thread, i.e. we need to leave some room for these);
- It's reactor is allocated to the `static` context eagerly as its constructor function is `const` (hence no stack blowups);
- The reactor has a smaller memory footprint too (~ 500 bytes), as it is hard-coded to the `select` syscall and does not support timers. MCUs (with lwIP) usually have max file and socket handles in the lower tens (~ 20 in ESP-IDF) so all structures can be limited to that size;
- No heap allocations - initially and during polling.

Further, `async-io` has a non-trivial set of dependencies (again - for MCUs; for regular OSes it is a dwarf by any meaningful measurement!): `rustix`, `polling`, `async-lock`, `event`, `tracing`, `parking-lot` and more. Nothing wrong with with that per-se, but that's a large implementation surface that e.g. recently is triggering a possible miscompilation on Espressif xtensa targets (NOT that this is a justification not to root-cause and fix the problem!).

`async-io-mini` only has the following non-optional dependencies:
- `libc` (which indirectly comes with Rust STD anyway);
- `heapless` (for `heapless::Vec` and nothing else);
- `log` (might become optional);
- `enumset` (not crucial, might remove).

## Limitations

### No timers

`async-io-mini` does NOT have an equivalent of `async_io::Timer`. On ESP-IDF at least, timers based on OS systicks are often not very useful, as the OS systick is low-res (10ms).

Workaround: use the `Timer` struct from the [`embassy-time`](https://crates.io/crates/embassy-time) crate, which provides a very similar API and is highly optimized for embedded environments. On the ESP-IDF, the `embassy-time-driver` implementation is backed by the ESP-IDF Timer service, which runs off from a high priority thread by default and thus has good res.

### No equivalent of `async_io::block_on`

Implementing socket polling as a shared task between the hidden `async-io-mini` thread and the thread calling `async_io_mini::block_on` is not trivial and probably not worth it on MCUs. Just use `futures_lite::block_on` or the `block_on` equivalent for your OS (i.e. `esp_idf_svc::hal::task::block_on` for the ESP-IDF).

## Implementation

The first time `Async` is used, a thread named `async-io-mini` will be spawned.
The purpose of this thread is to wait for I/O events reported by the operating system, and then
wake appropriate futures blocked on I/O when they can be resumed.

To wait for the next I/O event, the "async-io-mini" thread uses the [select](https://en.wikipedia.org/wiki/Select_(Unix)) syscall, and **is thus only useful for MCUs (might just be the ESP-IDF) where the number of file or socket handles is very small anyway**.

## Examples

Connect to `example.com:80`.

```rust
use async_io_mini::Async;
use futures_lite::{future::FutureExt, io};

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

let addr = "example.com:80".to_socket_addrs()?.next().unwrap();

let stream = Async::<TcpStream>::connect(addr).await?;
```

## License

Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

#### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
