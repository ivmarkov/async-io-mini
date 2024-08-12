use core::fmt::{self, Debug};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::time::Duration;

use std::time::Instant;

/// A future or stream that emits timed events.
///
/// Timers are futures that output a single [`Instant`] when they fire.
///
/// Timers are also streams that can output [`Instant`]s periodically.
///
/// # Precision
///
/// There is a limit on the maximum precision that a `Timer` can provide. This limit is
/// dependent on the current platform; for instance, on Windows, the maximum precision is
/// about 16 milliseconds. Because of this limit, the timer may sleep for longer than the
/// requested duration. It will never sleep for less.
///
/// # Examples
///
/// Sleep for 1 second:
///
/// ```
/// use async_io_mini::Timer;
/// use std::time::Duration;
///
/// # futures_lite::future::block_on(async {
/// Timer::after(Duration::from_secs(1)).await;
/// # });
/// ```
///
/// Timeout after 1 second:
///
/// ```
/// use async_io_mini::Timer;
/// use futures_lite::FutureExt;
/// use std::time::Duration;
///
/// # futures_lite::future::block_on(async {
/// let wait = core::future::pending::<Result<(), std::io::Error>>()
///     .or(async {
///         Timer::after(Duration::from_secs(1)).await;
///         Err(std::io::ErrorKind::TimedOut.into())
///     })
///     .await?;
/// # std::io::Result::Ok(()) });
/// ```
//#[derive(Debug)] TODO
pub struct Timer {
    when: Option<Instant>,
    period: Duration,
    waker: Option<Waker>,
}

impl Timer {
    /// Creates a timer that will never fire.
    ///
    /// # Examples
    ///
    /// This function may also be useful for creating a function with an optional timeout.
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_io_mini::Timer;
    /// use futures_lite::prelude::*;
    /// use std::time::Duration;
    ///
    /// async fn run_with_timeout(timeout: Option<Duration>) {
    ///     let timer = timeout
    ///         .map(|timeout| Timer::after(timeout))
    ///         .unwrap_or_else(Timer::never);
    ///
    ///     run_lengthy_operation().or(timer).await;
    /// }
    /// # // Note that since a Timer as a Future returns an Instant,
    /// # // this function needs to return an Instant to be used
    /// # // in "or".
    /// # async fn run_lengthy_operation() -> std::time::Instant {
    /// #    std::time::Instant::now()
    /// # }
    ///
    /// // Times out after 5 seconds.
    /// run_with_timeout(Some(Duration::from_secs(5))).await;
    /// // Does not time out.
    /// run_with_timeout(None).await;
    /// # });
    /// ```
    pub fn never() -> Timer {
        let _fix_linking = embassy_time::Timer::after(embassy_time::Duration::from_secs(1));

        Timer {
            when: None,
            period: Duration::MAX,
            waker: None,
        }
    }

    /// Creates a timer that emits an event once after the given duration of time.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_io_mini::Timer;
    /// use std::time::Duration;
    ///
    /// # futures_lite::future::block_on(async {
    /// Timer::after(Duration::from_secs(1)).await;
    /// # });
    /// ```
    pub fn after(duration: Duration) -> Timer {
        let Some(start) = Instant::now().checked_add(duration) else {
            return Timer::never();
        };

        Timer::interval_at(start, Duration::MAX)
    }

    /// Creates a timer that emits an event once at the given time instant.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_io_mini::Timer;
    /// use std::time::{Duration, Instant};
    ///
    /// # futures_lite::future::block_on(async {
    /// let now = Instant::now();
    /// let when = now + Duration::from_secs(1);
    /// Timer::at(when).await;
    /// # });
    /// ```
    pub fn at(instant: Instant) -> Timer {
        Timer::interval_at(instant, Duration::MAX)
    }

    /// Creates a timer that emits events periodically.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_io_mini::Timer;
    /// use futures_lite::StreamExt;
    /// use std::time::{Duration, Instant};
    ///
    /// # futures_lite::future::block_on(async {
    /// let period = Duration::from_secs(1);
    /// Timer::interval(period).next().await;
    /// # });
    /// ```
    pub fn interval(period: Duration) -> Timer {
        let Some(start) = Instant::now().checked_add(period) else {
            return Timer::never();
        };

        Timer::interval_at(start, period)
    }

    /// Creates a timer that emits events periodically, starting at `start`.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_io_mini::Timer;
    /// use futures_lite::StreamExt;
    /// use std::time::{Duration, Instant};
    ///
    /// # futures_lite::future::block_on(async {
    /// let start = Instant::now();
    /// let period = Duration::from_secs(1);
    /// Timer::interval_at(start, period).next().await;
    /// # });
    /// ```
    pub fn interval_at(start: Instant, period: Duration) -> Timer {
        if Self::ticks(&start).is_some() {
            Timer {
                when: Some(start),
                period,
                waker: None,
            }
        } else {
            Timer::never()
        }
    }

    /// Indicates whether or not this timer will ever fire.
    ///
    /// [`never()`] will never fire, and timers created with [`after()`] or [`at()`] will fire
    /// if the duration is not too large.
    ///
    /// [`never()`]: Timer::never()
    /// [`after()`]: Timer::after()
    /// [`at()`]: Timer::at()
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_io_mini::Timer;
    /// use futures_lite::prelude::*;
    /// use std::time::Duration;
    ///
    /// // `never` will never fire.
    /// assert!(!Timer::never().will_fire());
    ///
    /// // `after` will fire if the duration is not too large.
    /// assert!(Timer::after(Duration::from_secs(1)).will_fire());
    /// assert!(!Timer::after(Duration::MAX).will_fire());
    ///
    /// // However, once an `after` timer has fired, it will never fire again.
    /// let mut t = Timer::after(Duration::from_secs(1));
    /// assert!(t.will_fire());
    /// (&mut t).await;
    /// assert!(!t.will_fire());
    ///
    /// // Interval timers will fire periodically.
    /// let mut t = Timer::interval(Duration::from_secs(1));
    /// assert!(t.will_fire());
    /// t.next().await;
    /// assert!(t.will_fire());
    /// # });
    /// ```
    #[inline]
    pub fn will_fire(&self) -> bool {
        self.when.is_some()
    }

    /// Sets the timer to emit an en event once after the given duration of time.
    ///
    /// Note that resetting a timer is different from creating a new timer because
    /// [`set_after()`][`Timer::set_after()`] does not remove the waker associated with the task
    /// that is polling the timer.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_io_mini::Timer;
    /// use std::time::Duration;
    ///
    /// # futures_lite::future::block_on(async {
    /// let mut t = Timer::after(Duration::from_secs(1));
    /// t.set_after(Duration::from_millis(100));
    /// # });
    /// ```
    pub fn set_after(&mut self, duration: Duration) {
        match Instant::now().checked_add(duration) {
            Some(instant) => self.set_at(instant),
            // Overflow to never going off.
            None => self.set_never(),
        }
    }

    /// Sets the timer to emit an event once at the given time instant.
    ///
    /// Note that resetting a timer is different from creating a new timer because
    /// [`set_at()`][`Timer::set_at()`] does not remove the waker associated with the task
    /// that is polling the timer.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_io_mini::Timer;
    /// use std::time::{Duration, Instant};
    ///
    /// # futures_lite::future::block_on(async {
    /// let mut t = Timer::after(Duration::from_secs(1));
    ///
    /// let now = Instant::now();
    /// let when = now + Duration::from_secs(1);
    /// t.set_at(when);
    /// # });
    /// ```
    pub fn set_at(&mut self, instant: Instant) {
        let ticks = Self::ticks(&instant);

        if let Some(ticks) = ticks {
            self.when = Some(instant);
            self.period = Duration::MAX;

            if let Some(waker) = self.waker.as_ref() {
                embassy_time_queue_driver::schedule_wake(ticks, waker);
            }
        } else {
            self.set_never();
        }
    }

    /// Sets the timer to emit events periodically.
    ///
    /// Note that resetting a timer is different from creating a new timer because
    /// [`set_interval()`][`Timer::set_interval()`] does not remove the waker associated with the
    /// task that is polling the timer.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_io_mini::Timer;
    /// use futures_lite::StreamExt;
    /// use std::time::{Duration, Instant};
    ///
    /// # futures_lite::future::block_on(async {
    /// let mut t = Timer::after(Duration::from_secs(1));
    ///
    /// let period = Duration::from_secs(2);
    /// t.set_interval(period);
    /// # });
    /// ```
    pub fn set_interval(&mut self, period: Duration) {
        match Instant::now().checked_add(period) {
            Some(instant) => self.set_interval_at(instant, period),
            // Overflow to never going off.
            None => self.set_never(),
        }
    }

    /// Sets the timer to emit events periodically, starting at `start`.
    ///
    /// Note that resetting a timer is different from creating a new timer because
    /// [`set_interval_at()`][`Timer::set_interval_at()`] does not remove the waker associated with
    /// the task that is polling the timer.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_io_mini::Timer;
    /// use futures_lite::StreamExt;
    /// use std::time::{Duration, Instant};
    ///
    /// # futures_lite::future::block_on(async {
    /// let mut t = Timer::after(Duration::from_secs(1));
    ///
    /// let start = Instant::now();
    /// let period = Duration::from_secs(2);
    /// t.set_interval_at(start, period);
    /// # });
    /// ```
    pub fn set_interval_at(&mut self, start: Instant, period: Duration) {
        let ticks = Self::ticks(&start);

        if let Some(ticks) = ticks {
            self.when = Some(start);
            self.period = period;

            if let Some(waker) = self.waker.as_ref() {
                embassy_time_queue_driver::schedule_wake(ticks, waker);
            }
        } else {
            // Overflow to never going off.
            self.set_never();
        }
    }

    fn set_never(&mut self) {
        self.when = None;
        self.waker = None;
        self.period = Duration::MAX;
    }

    fn fired_at(&mut self, cx: &mut Context<'_>) -> Option<Instant> {
        let when = self.when?;

        if when > Instant::now() {
            let ticks = Self::ticks(&when);

            if let Some(ticks) = ticks {
                if self
                    .waker
                    .as_ref()
                    .map(|waker| !waker.will_wake(cx.waker()))
                    .unwrap_or(true)
                {
                    self.waker = Some(cx.waker().clone());
                    embassy_time_queue_driver::schedule_wake(ticks, cx.waker());
                }
            } else {
                self.set_never();
            }

            None
        } else {
            Some(when)
        }
    }

    fn ticks(instant: &Instant) -> Option<u64> {
        fn duration_ticks(duration: &Duration) -> Option<u64> {
            let ticks = duration.as_secs() as u128 * embassy_time_driver::TICK_HZ as u128
                + duration.subsec_nanos() as u128 * embassy_time_driver::TICK_HZ as u128
                    / 1_000_000_000;

            u64::try_from(ticks).ok()
        }

        let now = Instant::now();
        let now_ticks = embassy_time_driver::now();

        if *instant >= now {
            let dur_ticks = duration_ticks(&instant.duration_since(now));

            dur_ticks.and_then(|dur_ticks| now_ticks.checked_add(dur_ticks))
        } else {
            let dur_ticks = duration_ticks(&now.duration_since(*instant));

            dur_ticks.map(|dur_ticks| now_ticks.saturating_sub(dur_ticks))
        }
    }
}

impl Debug for Timer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Timer")
            .field("start", &self.when.as_ref())
            .field("period", &self.period)
            .finish()
    }
}

impl Future for Timer {
    type Output = Instant;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Some(when) = self.fired_at(cx) else {
            return Poll::Pending;
        };

        self.set_never();

        Poll::Ready(when)
    }
}

#[cfg(feature = "futures-lite")]
impl futures_lite::Stream for Timer {
    type Item = Instant;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(when) = self.fired_at(cx) else {
            return Poll::Pending;
        };

        let next_when = when.checked_add(self.period);

        if let Some(next_when) = next_when {
            let period = self.period;

            self.set_interval_at(next_when, period);
        } else {
            self.set_never();
        }

        Poll::Ready(Some(when))
    }
}
