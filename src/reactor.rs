use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::Waker;

use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use enumset::{EnumSet, EnumSetType};

use log::debug;

use libc as sys;

const MAX_REGISTRATIONS: usize = 20;

//const FD_SEGMENT: usize = sys::FD_SETSIZE / core::mem::size_of::<sys::fd_set>();

#[macro_export]
macro_rules! syscall {
    ($ret:expr) => {{
        if $ret < 0 {
            Err(::std::io::Error::from_raw_os_error($ret))
        } else {
            Ok($ret)
        }
    }};
}

#[macro_export]
macro_rules! syscall_los {
    ($ret:expr) => {{
        if $ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok($ret)
        }
    }};
}

#[macro_export]
macro_rules! syscall_los_eagain {
    ($ret:expr) => {{
        #[allow(unreachable_patterns)]
        match syscall_los!($ret) {
            Ok(_) => Ok(()),
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(sys::EINPROGRESS) | Some(sys::EAGAIN) | Some(sys::EWOULDBLOCK)
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }?;

        Ok::<_, io::Error>(())
    }};
}

#[derive(EnumSetType, Debug)]
pub(crate) enum Event {
    Read = 0,
    Write = 1,
}

struct Fds {
    read: MaybeUninit<sys::fd_set>,
    write: MaybeUninit<sys::fd_set>,
    except: MaybeUninit<sys::fd_set>,
}

impl Fds {
    const fn new() -> Self {
        Self {
            read: MaybeUninit::uninit(),
            write: MaybeUninit::uninit(),
            except: MaybeUninit::uninit(),
        }
    }

    fn zero(&mut self) {
        unsafe {
            sys::FD_ZERO(self.read.as_mut_ptr());
            sys::FD_ZERO(self.write.as_mut_ptr());
            sys::FD_ZERO(self.except.as_mut_ptr());
        }
    }

    fn is_set(&self, fd: RawFd, event: Event) -> bool {
        unsafe { sys::FD_ISSET(fd, self.fd_set(event)) }
    }

    fn set(&mut self, fd: RawFd, event: Event) {
        unsafe { sys::FD_SET(fd, self.fd_set_mut(event)) }
    }

    fn fd_set(&self, event: Event) -> &sys::fd_set {
        unsafe {
            match event {
                Event::Read => self.read.assume_init_ref(),
                Event::Write => self.write.assume_init_ref(),
            }
        }
    }

    fn fd_set_mut(&mut self, event: Event) -> &mut sys::fd_set {
        unsafe {
            match event {
                Event::Read => self.read.assume_init_mut(),
                Event::Write => self.write.assume_init_mut(),
            }
        }
    }
}

struct Registration {
    fd: RawFd,
    events: EnumSet<Event>,
    wakers: [Option<Waker>; 2],
}

struct Registrations<const N: usize> {
    vec: heapless::Vec<Registration, N>,
    event_fd: Option<OwnedFd>,
}

impl<const N: usize> Registrations<N> {
    const fn new() -> Self {
        Self {
            vec: heapless::Vec::new(),
            event_fd: None,
        }
    }

    fn register(&mut self, fd: RawFd) -> io::Result<()> {
        if fd < 0
            || self
                .event_fd
                .as_ref()
                .map(|event_fd| fd == event_fd.as_raw_fd())
                .unwrap_or(false)
        {
            Err(ErrorKind::InvalidInput)?;
        }

        if fd >= sys::FD_SETSIZE as RawFd {
            Err(ErrorKind::OutOfMemory)?;
        }

        if self.vec.iter().any(|reg| reg.fd == fd) {
            Err(ErrorKind::InvalidInput)?;
        }

        self.vec
            .push(Registration {
                fd,
                events: EnumSet::empty(),
                wakers: [None, None],
            })
            .map_err(|_| ErrorKind::OutOfMemory)?;

        Ok(())
    }

    fn deregister(&mut self, fd: RawFd) -> io::Result<()> {
        let Some(index) = self.vec.iter_mut().position(|reg| reg.fd == fd) else {
            return Err(ErrorKind::NotFound.into());
        };

        self.vec.swap_remove(index);

        self.notify()?;

        Ok(())
    }

    fn set(&mut self, fd: RawFd, event: Event, waker: &Waker) -> io::Result<()> {
        let Some(registration) = self.vec.iter_mut().find(|reg| reg.fd == fd) else {
            return Err(ErrorKind::NotFound.into());
        };

        registration.events.remove(event);

        if let Some(prev_waker) = registration.wakers[event as usize].replace(waker.clone()) {
            if !prev_waker.will_wake(waker) {
                prev_waker.wake();
            }
        }

        self.notify()?;

        Ok(())
    }

    fn fetch(&mut self, fd: RawFd, event: Event) -> io::Result<bool> {
        let Some(registration) = self.vec.iter_mut().find(|reg| reg.fd == fd) else {
            return Err(ErrorKind::NotFound.into());
        };

        let set = registration.events.contains(event);

        registration.events.remove(event);

        Ok(set)
    }

    fn set_fds(&self, fds: &mut Fds) -> io::Result<Option<RawFd>> {
        fds.zero();

        let mut max: Option<RawFd> = None;

        if let Some(event_fd) = self.event_fd.as_ref().map(|event_fd| event_fd.as_raw_fd()) {
            fds.set(event_fd, Event::Read);
            max = Some(max.map_or(event_fd, |max| max.max(event_fd)));

            debug!("Set event FD: {event_fd}");
        }

        for registration in &self.vec {
            for event in EnumSet::ALL {
                if registration.wakers[event as usize].is_some() {
                    fds.set(registration.fd, event);

                    debug!("Set registration FD: {}/{event:?}", registration.fd);
                }

                max = Some(max.map_or(registration.fd, |max| max.max(registration.fd)));
            }
        }

        debug!("Max FDs: {max:?}");

        Ok(max)
    }

    fn update_events(&mut self, fds: &Fds) -> io::Result<()> {
        debug!("Updating events");

        self.consume_notification()?;

        for registration in &mut self.vec {
            for event in EnumSet::ALL {
                if fds.is_set(registration.fd, event) {
                    debug!("Registration FD is set: {}/{event:?}", registration.fd);

                    registration.events |= event;
                    if let Some(waker) = registration.wakers[event as usize].take() {
                        waker.wake();
                    }
                }
            }
        }

        Ok(())
    }

    fn create_notification(&mut self) -> io::Result<bool> {
        if self.event_fd.is_none() {
            #[cfg(not(target_os = "espidf"))]
            let event_fd =
                unsafe { OwnedFd::from_raw_fd(syscall_los!(sys::eventfd(0, sys::EFD_NONBLOCK))?) };

            // Note that the eventfd() implementation in ESP-IDF deviates from the specification in the following ways:
            // 1) The file descriptor is always in a non-blocking mode, as if EFD_NONBLOCK was passed as a flag;
            //    passing EFD_NONBLOCK or calling fcntl(.., F_GETFL/F_SETFL) on the eventfd() file descriptor is not supported
            // 2) It always returns the counter value, even if it is 0. This is contrary to the specification which mandates
            //    that it should instead fail with EAGAIN
            //
            // (1) is not a problem for us, as we want the eventfd() file descriptor to be in a non-blocking mode anyway
            // (2) is also not a problem, as long as we don't try to read the counter value in an endless loop when we detect being notified
            #[cfg(target_os = "espidf")]
            let event_fd = unsafe {
                OwnedFd::from_raw_fd(syscall_los!(sys::eventfd(0, 0)).map_err(|err| {
                    match err {
                        err if err.kind() == io::ErrorKind::PermissionDenied => {
                            // EPERM can happen if the eventfd isn't initialized yet.
                            // Tell the user to call esp_vfs_eventfd_register.
                            io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "failed to initialize eventfd for polling, try calling `esp_vfs_eventfd_register`"
                            )
                        },
                        err => err,
                    }
                })?)
            };

            debug!("Created event FD: {}", event_fd.as_raw_fd());

            self.event_fd = Some(event_fd);

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn destroy_notification(&mut self) -> io::Result<bool> {
        if let Some(event_fd) = self.event_fd.take() {
            syscall!(unsafe { sys::close(event_fd.as_raw_fd()) })?;

            debug!("Closed event FD: {}", event_fd.as_raw_fd());

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn notify(&self) -> io::Result<bool> {
        if let Some(event_fd) = self.event_fd.as_ref() {
            let event_fd = event_fd.as_raw_fd();

            syscall_los_eagain!(unsafe {
                sys::write(
                    event_fd,
                    &u64::to_be_bytes(1_u64) as *const _ as *const _,
                    core::mem::size_of::<u64>(),
                )
            })?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn consume_notification(&mut self) -> io::Result<bool> {
        if let Some(event_fd) = self.event_fd.as_ref() {
            let event_fd = event_fd.as_raw_fd();

            let mut buf = [0_u8; core::mem::size_of::<u64>()];

            syscall_los_eagain!(unsafe {
                sys::read(
                    event_fd,
                    &mut buf as *mut _ as *mut _,
                    core::mem::size_of::<u64>(),
                )
            })?;

            debug!("Consumed notification");

            Ok(true)
        } else {
            Ok(false)
        }
    }
}

pub struct Reactor<const N: usize> {
    registrations: std::sync::Mutex<Registrations<N>>,
    started: AtomicBool,
}

impl<const N: usize> Reactor<N> {
    const fn new() -> Self {
        Self {
            registrations: std::sync::Mutex::new(Registrations::new()),
            started: AtomicBool::new(false),
        }
    }

    /// Starts the reactor. Returns `false` if it had been already started.
    pub fn start(&'static self) -> io::Result<bool> {
        if self.started.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }

        std::thread::Builder::new()
            .name("async-io-mini".into())
            .stack_size(3048)
            .spawn(move || {
                self.run().unwrap();
            })?;

        Ok(true)
    }

    pub(crate) fn register(&self, fd: RawFd) -> io::Result<()> {
        self.lock(|regs| regs.register(fd))
    }

    pub(crate) fn deregister(&self, fd: RawFd) -> io::Result<()> {
        self.lock(|regs| regs.deregister(fd))
    }

    // pub(crate) fn set(&self, fd: RawFd, event: Event, waker: &Waker) -> io::Result<()> {
    //     self.lock(|regs| regs.set(fd, event, waker))
    // }

    pub(crate) fn fetch(&self, fd: RawFd, event: Event) -> io::Result<bool> {
        self.lock(|regs| regs.fetch(fd, event))
    }

    pub(crate) fn fetch_or_set(&self, fd: RawFd, event: Event, waker: &Waker) -> io::Result<bool> {
        self.lock(|regs| {
            if regs.fetch(fd, event)? {
                Ok(true)
            } else {
                regs.set(fd, event, waker)?;

                Ok(false)
            }
        })
    }

    fn run(&self) -> io::Result<()> {
        if !self.lock(Registrations::create_notification)? {
            Err(ErrorKind::AlreadyExists)?;
        }

        debug!("Running");

        let result = loop {
            let result = self.wait();

            if result.is_err() {
                break result;
            }
        };

        if !self.lock(Registrations::destroy_notification)? {
            Err(ErrorKind::NotFound)?;
        }

        result
    }

    fn wait(&self) -> io::Result<()> {
        let mut fds = Fds::new();

        if let Some(max) = self.lock(|inner| inner.set_fds(&mut fds))? {
            debug!("Start select");

            syscall_los!(unsafe {
                sys::select(
                    max + 1,
                    fds.read.assume_init_mut(),
                    fds.write.assume_init_mut(),
                    fds.except.assume_init_mut(),
                    core::ptr::null_mut(),
                )
            })?;

            debug!("End select");

            self.lock(|inner| inner.update_events(&fds))?;
        }

        Ok(())
    }

    fn lock<F, R>(&self, f: F) -> io::Result<R>
    where
        F: FnOnce(&mut Registrations<N>) -> io::Result<R>,
    {
        let mut inner = self.registrations.lock().unwrap();

        f(&mut inner)
    }
}

pub static REACTOR: Reactor<MAX_REGISTRATIONS> = Reactor::new();
