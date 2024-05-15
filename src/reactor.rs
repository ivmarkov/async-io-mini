use core::task::Waker;

use std::io::{self, ErrorKind};
use std::mem::MaybeUninit;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::Thread;

use enumset::{EnumSet, EnumSetType};
use log::error;

//use log::info;

// use crate::hal::cpu::Core;
// use crate::hal::task::CriticalSection;
// use crate::sys::esp;
// use crate::sys::fd_set;

use libc as sys;

const MAX_REGISTRATIONS: usize = 20;

const FD_SEGMENT: usize = sys::FD_SETSIZE / core::mem::size_of::<sys::fd_set>();

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
    event_fd: Option<RawFd>,
}

impl<const N: usize> Registrations<N> {
    const fn new() -> Self {
        Self {
            vec: heapless::Vec::new(),
            event_fd: None,
        }
    }

    fn register(&mut self, fd: RawFd) -> io::Result<()> {
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

        if let Some(event_fd) = self.event_fd {
            fds.set(event_fd, Event::Read);
            max = Some(max.map_or(event_fd, |max| max.max(event_fd)));
            error!("Event fd set: {event_fd}");
        }

        for registration in &self.vec {
            for event in EnumSet::ALL {
                if registration.wakers[event as usize].is_some() {
                    error!("Registration fd set: {event:?} {}", registration.fd);
                    fds.set(registration.fd, event);
                }

                max = Some(max.map_or(registration.fd, |max| max.max(registration.fd)));
            }
        }

        error!("MAX: {max:?}");
        Ok(max)
    }

    fn update_events(&mut self, fds: &Fds) -> io::Result<()> {
        self.consume_notification()?;

        error!("CONSUMED");
        // unsafe {
        // error!("READ: {:?}, WRITE: {:?}", fds.read.assume_init_ref(), fds.write.assume_init_ref());
        // }
        for registration in &mut self.vec {
            for event in EnumSet::ALL {
                if fds.is_set(registration.fd, event) {
                    error!("FD SET: {event:?} {}", registration.fd);

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
            let handle = unsafe { sys::eventfd(0, sys::O_NONBLOCK as _) };

            self.event_fd = Some(handle);

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn destroy_notification(&mut self) -> io::Result<bool> {
        if let Some(event_fd) = self.event_fd.take() {
            syscall!(unsafe { sys::close(event_fd) })?;

            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn notify(&self) -> io::Result<bool> {
        if let Some(event_fd) = self.event_fd {
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
        if let Some(event_fd) = self.event_fd {
            let mut buf = [0_u8; core::mem::size_of::<u64>()];

            syscall_los_eagain!(unsafe {
                sys::read(
                    event_fd,
                    &mut buf as *mut _ as *mut _,
                    core::mem::size_of::<u64>(),
                )
            })?;

            Ok(true)
        } else {
            Ok(false)
        }
    }
}

// /// Wake runner configuration
// #[derive(Clone, Debug)]
// pub struct VfsReactorConfig {
//     pub task_name: &'static CStr,
//     pub task_stack_size: usize,
//     pub task_priority: u8,
//     pub task_pin_to_core: Option<Core>,
// }

// impl VfsReactorConfig {
//     pub const fn new() -> Self {
//         Self {
//             task_name: unsafe { CStr::from_bytes_with_nul_unchecked(b"VfsReactor\0") },
//             task_stack_size: 1024,
//             task_priority: 9,
//             task_pin_to_core: None,
//         }
//     }
// }

// impl Default for VfsReactorConfig {
//     fn default() -> Self {
//         Self::new()
//     }
// }

pub struct VfsReactor<const N: usize> {
    registrations: std::sync::Mutex<Registrations<N>>,
    started: AtomicBool,
    // task_cs: CriticalSection,
    // task: AtomicPtr<crate::sys::tskTaskControlBlock>,
    // task_config: VfsReactorConfig,
}

impl<const N: usize> VfsReactor<N> {
    const fn new() -> Self {
        Self {
            registrations: std::sync::Mutex::new(Registrations::new()),
            // task_cs: CriticalSection::new(),
            // task: AtomicPtr::new(core::ptr::null_mut()),
            // task_config: config,
            started: AtomicBool::new(false),
        }
    }

    // /// Returns `true` if the wake runner is started.
    // pub fn is_started(&self) -> bool {
    //     !self.task.load(Ordering::SeqCst).is_null()
    // }

    /// Starts the wake runner. Returns `false` if it had been already started.
    pub fn start(&'static self) -> io::Result<bool> {
        if self.started.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }

        std::thread::spawn(move || {
            if let Err(e) = self.run() {
                panic!();
                //error!("IsrReactor {:?} error: {:?}", self.task_config.task_name, e);
            }
        });

        // let _guard = self.task_cs.enter();

        // if self.task.load(Ordering::SeqCst).is_null() {
        //     let task = unsafe {
        //         crate::hal::task::create(
        //             Self::task_run,
        //             self.task_config.task_name,
        //             self.task_config.task_stack_size,
        //             self as *const _ as *const c_void as *mut _,
        //             self.task_config.task_priority,
        //             self.task_config.task_pin_to_core,
        //         )
        //         .map_err(|e| io::Error::from_raw_os_error(e.code()))?
        //     };

        //     self.task.store(task as _, Ordering::SeqCst);

        //     info!("IsrReactor {:?} started.", self.task_config.task_name);

        //     Ok(true)
        // } else {
        //     Ok(false)
        // }
        Ok(true)
    }

    // /// Stops the wake runner. Returns `false` if it had been already stopped.
    // pub fn stop(&self) -> bool {
    //     let _guard = self.task_cs.enter();

    //     let task = self.task.swap(core::ptr::null_mut(), Ordering::SeqCst);

    //     if !task.is_null() {
    //         unsafe {
    //             crate::hal::task::destroy(task as _);
    //         }

    //         info!("IsrReactor {:?} stopped.", self.task_config.task_name);

    //         true
    //     } else {
    //         false
    //     }
    // }

    // extern "C" fn task_run(ctx: *mut c_void) {
    //     let this =
    //         unsafe { (ctx as *mut VfsReactor<N> as *const VfsReactor<N>).as_ref() }.unwrap();

    //     this.run();
    // }

    pub(crate) fn register(&self, fd: RawFd) -> io::Result<()> {
        self.lock(|regs| regs.register(fd))
    }

    pub(crate) fn deregister(&self, fd: RawFd) -> io::Result<()> {
        self.lock(|regs| regs.deregister(fd))
    }

    pub(crate) fn set(&self, fd: RawFd, event: Event, waker: &Waker) -> io::Result<()> {
        self.lock(|regs| regs.set(fd, event, waker))
    }

    pub(crate) fn fetch(&self, fd: RawFd, event: Event) -> io::Result<bool> {
        self.lock(|regs| regs.fetch(fd, event))
    }

    fn run(&self) -> io::Result<()> {
        error!("RUNNING");
        if !self.lock(Registrations::create_notification)? {
            Err(ErrorKind::AlreadyExists)?;
        }

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
            error!("SELECTING");
            syscall_los!(unsafe {
                sys::select(
                    max + 1,
                    fds.read.assume_init_mut(),
                    fds.write.assume_init_mut(),
                    fds.except.assume_init_mut(),
                    core::ptr::null_mut(),
                )
            })?;

            error!("SELECTING OUTCOME");
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

pub static VFS_REACTOR: VfsReactor<MAX_REGISTRATIONS> = VfsReactor::new();
