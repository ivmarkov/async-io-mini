# async-io-mini

[![CI](https://github.com/ivmarkov/async-io-mini/actions/workflows/ci.yml/badge.svg)](https://github.com/ivmarkov/async-io-mini/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](https://github.com/ivmarkov/async-io-mini)
[![Cargo](https://img.shields.io/crates/v/async-io-mini.svg)](https://crates.io/crates/async-io-mini)
[![Documentation](https://docs.rs/async-io/badge.svg)](https://docs.rs/async-io-mini)

Async I/O and timers for MCUs.

This crate is a fork of the splendid [`async-io`](https://github.com/smol-rs/async-io) crate targetting MCUs and ESP-IDF in particular.

## How to use?

`async-io-mini` is an API-compatible replacement for the `Async` and `Timer` types from `async-io`.

So either:
* Just replace all `use async_io` occurances in your crate with `use async_io_mini`
* Or - in your `Cargo.toml` - replace:
  * `async-io = "..."`
  * with `async-io = { package = "async-io-mini", ... }`

Additionally, you need to provide an `embassy-time-driver` implementation. This is either done by the HAL of your MCU, or `embassy-time` provides you with a `std`-specific implementation. If you are not using `embassy-executor`, you will also need to select one of the `embassy-time/generic-queue-*` features.

## Justification

While `async-io` supports a ton of operating systems - _including ESP-IDF for the Espressif MCU chips_ - it does have a non-trivial memory consumption in the hidden thread named `async-io`.  Since its hidden `Reactor` object is initialized lazily, it so happens that it is first allocated on-stack, and then it is moved into the static context. This requires the `async-io` thread (as well as _any_ thread from where you are polling sockets) to have at least 8K stack, which - by MCU standards! - is relatively large if you are memory-constrained.

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

## Enhancements

The `Timer` type of `async_io_mini` is based on the `embassy-time` crate, and as such should offer a higher resolution on embedded operating systems like the ESP-IDF than what can be normally achieved by implementing timers using the `timeout` parameter of the `select` syscall (as `async-io` does).

The reason for this is that on the ESP-IDF, the `timeout` parameter of `select` provides a resolution of 10ms (one FreeRTOS sys-tick), while
`embassy-time` is implemented using the [ESP-IDF Timer service](https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/esp_timer.html), which provides resolutions up to 1 microsecond.

With that said, for greenfield code that does not need to be compatible with `async-io`, use the native `embassy_time::Timer` and `embassy_time::Ticker` rather than `async_io_mini::Timer`, because the latter has a larger memory footprint (40 bytes on 32bit archs) compared to the `embassy-time` types (8 and 16 bytes each).

## Limitations

### No equivalent of `async_io::block_on`

Implementing socket polling as a shared task between the hidden `async-io-mini` thread and the thread calling `async_io_mini::block_on` is not trivial and probably not worth it on MCUs. Just use `futures_lite::block_on` or the `block_on` equivalent for your OS (i.e. `esp_idf_svc::hal::task::block_on` for the ESP-IDF).

## Implementation

### Async

The first time `Async` is used, a thread named `async-io-mini` will be spawned.
The purpose of this thread is to wait for I/O events reported by the operating system, and then
wake appropriate futures blocked on I/O when they can be resumed.

To wait for the next I/O event, the "async-io-mini" thread uses the [select](https://en.wikipedia.org/wiki/Select_(Unix)) syscall, and **is thus only useful for MCUs (might just be the ESP-IDF) where the number of file or socket handles is very small anyway**.

### Timer

As per above, the `Timer` type is a wrapper around the functionality provided by the `embassy-time` crate.

## Examples

Connect to `example.com:80`, or time out after 10 seconds.

```rust
use async_io_mini::{Async, Timer};
use futures_lite::{future::FutureExt, io};

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

let addr = "example.com:80".to_socket_addrs()?.next().unwrap();

let stream = Async::<TcpStream>::connect(addr).or(async {
    Timer::after(Duration::from_secs(10)).await;
    Err(io::ErrorKind::TimedOut.into())
})
.await?;
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
