//! Keeps the wrapper alive while nextest signals the test's process group.

use std::{
    io::{self, Read, Write},
    mem::MaybeUninit,
    net::Shutdown,
    os::unix::{
        net::UnixStream,
        process::{CommandExt, ExitStatusExt},
    },
    process::{Command, ExitStatus},
};

const CANCELLATION: [i32; 4] = [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM];

pub fn pending() -> io::Result<Option<i32>> {
    let mut set = MaybeUninit::uninit();
    // SAFETY: sigpending initializes the set on success.
    let set = unsafe {
        if libc::sigpending(set.as_mut_ptr()) == -1 {
            return Err(io::Error::last_os_error());
        }
        set.assume_init()
    };
    Ok(CANCELLATION.into_iter().find(|&sig| {
        // SAFETY: set is initialized and sig is a valid signal number.
        unsafe { libc::sigismember(&set, sig) == 1 }
    }))
}

pub struct StartGate(UnixStream);

pub fn prepare(command: &mut Command) -> io::Result<StartGate> {
    // This is the wrapper's only child. All wrapper threads inherit the blocked
    // mask until exit, so nextest can still escalate while we wait or drain output.
    let original = set_mask(libc::SIG_BLOCK, &signal_set(&CANCELLATION)?)?;

    let (parent, mut child) = UnixStream::pair()?;
    // A pending signal isn't inherited across fork. Ask the parent once we're
    // in the process group. From then on, group signals reach the child directly.
    // SAFETY: after fork this hook uses only socket I/O, signal operations and
    // errno-based errors. Neither a lock nor an allocator is needed.
    unsafe {
        command.pre_exec(move || {
            child.write_all(&[1])?;
            let mut reply = [0];
            child.read_exact(&mut reply)?;
            if reply[0] != 0 {
                return die_by_signal(i32::from(reply[0]));
            }
            set_mask(libc::SIG_SETMASK, &original)?;
            Ok(())
        });
    }
    Ok(StartGate(parent))
}

impl StartGate {
    pub fn release(mut self) -> io::Result<()> {
        let mut ready = [0];
        if self.0.read(&mut ready)? != 0 {
            let signal = pending()?.map_or(0, |s| s as u8);
            self.0.write_all(&[signal])?;
        }
        Ok(())
    }
}

impl Drop for StartGate {
    fn drop(&mut self) {
        // Also wake a waiting child if we couldn't send the reply.
        let _ = self.0.shutdown(Shutdown::Write);
    }
}

pub fn exit(status: ExitStatus) -> anyhow::Result<()> {
    if let Some(code) = status.code() {
        std::process::exit(code);
    }
    Ok(die_by_signal(
        status.signal().expect("child exited or was signaled"),
    )?)
}

fn die_by_signal(number: i32) -> io::Result<()> {
    // SAFETY: install the system default for an OS-provided signal.
    unsafe {
        if number != libc::SIGKILL && libc::signal(number, libc::SIG_DFL) == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }
        set_mask(libc::SIG_UNBLOCK, &signal_set(&[number])?)?;
        if libc::raise(number) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Err(io::Error::from_raw_os_error(libc::EINVAL))
}

fn signal_set(numbers: &[i32]) -> io::Result<libc::sigset_t> {
    let mut set = MaybeUninit::uninit();
    // SAFETY: sigemptyset initializes the set before we add any signals.
    unsafe {
        if libc::sigemptyset(set.as_mut_ptr()) == -1 {
            return Err(io::Error::last_os_error());
        }
        let mut set = set.assume_init();
        for &number in numbers {
            if libc::sigaddset(&mut set, number) == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(set)
    }
}

fn set_mask(how: i32, set: &libc::sigset_t) -> io::Result<libc::sigset_t> {
    let mut old = MaybeUninit::uninit();
    // SAFETY: pthread_sigmask writes the previous mask on success.
    unsafe {
        let error = libc::pthread_sigmask(how, set, old.as_mut_ptr());
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        Ok(old.assume_init())
    }
}
