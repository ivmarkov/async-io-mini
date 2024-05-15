# async-io-mini

[![Build](https://github.com/ivmarkov/async-io-mini/workflows/Build%20and%20test/badge.svg)](
https://github.com/ivmarkov/async-io-mini/actions)
[![License](https://img.shields.io/badge/license-Apache--2.0_OR_MIT-blue.svg)](
https://github.com/ivmarkov/async-io-mini)
[![Cargo](https://img.shields.io/crates/v/async-io-mini.svg)](
https://crates.io/crates/async-io-mini)
[![Documentation](https://docs.rs/async-io/badge.svg)](
https://docs.rs/async-io-mini)

Async I/O.

This crate is an **experimental** fork of the splendid [`async-io`](https://github.com/smol-rs/async-io) crate targetting MCUs and ESP IDF in particular.

**TBD**: Justification coming. But all still experimental and might be retired soon.

## Implementation

The first time `Async` is used, a thread named "async-io-mini" will be spawned.
The purpose of this thread is to wait for I/O events reported by the operating system, and then
wake appropriate futures blocked on I/O or timers when they can be resumed.

To wait for the next I/O event, the "async-io-mini" thread uses [epoll] on Linux/Android/illumos,
[kqueue] on macOS/iOS/BSD, [event ports] on illumos/Solaris, and [IOCP] on Windows. That
functionality is provided by the [`polling`] crate.

However, note that you can also process I/O events and wake futures on any thread using the
`block_on()` function. The "async-io" thread is therefore just a fallback mechanism
processing I/O events in case no other threads are.

[epoll]: https://en.wikipedia.org/wiki/Epoll
[kqueue]: https://en.wikipedia.org/wiki/Kqueue
[event ports]: https://illumos.org/man/port_create
[IOCP]: https://learn.microsoft.com/en-us/windows/win32/fileio/i-o-completion-ports
[`polling`]: https://docs.rs/polling

## Examples

Connect to `example.com:80`, or time out after 10 seconds.

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
