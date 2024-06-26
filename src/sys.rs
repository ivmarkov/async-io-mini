pub use sys::*;

use libc as sys;

#[macro_export]
macro_rules! syscall {
    ($ret:expr) => {{
        let result = $ret;

        if result != 0 {
            Err(::std::io::Error::from_raw_os_error(result))
        } else {
            Ok(result)
        }
    }};
}

#[macro_export]
macro_rules! syscall_los {
    ($ret:expr) => {{
        let result = $ret;

        if result == -1 {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok(result)
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

#[macro_export]
macro_rules! ready {
    ($e:expr $(,)?) => {
        match $e {
            core::task::Poll::Ready(t) => t,
            core::task::Poll::Pending => return core::task::Poll::Pending,
        }
    };
}
