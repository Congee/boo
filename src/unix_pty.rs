#![cfg(target_os = "linux")]
#![allow(dead_code)]

use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::mpsc;

#[derive(Debug, Clone, Copy)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
    pub width_px: u16,
    pub height_px: u16,
}

impl PtySize {
    pub fn new(cols: u16, rows: u16, width_px: u16, height_px: u16) -> Self {
        Self {
            cols,
            rows,
            width_px,
            height_px,
        }
    }
}

pub struct PtyProcess {
    master_fd: RawFd,
    reader_fd: RawFd,
    child_pid: libc::pid_t,
    rx: mpsc::Receiver<Vec<u8>>,
    reaped: bool,
}

impl PtyProcess {
    pub fn spawn(
        command: Option<&CStr>,
        working_directory: Option<&Path>,
        size: PtySize,
    ) -> io::Result<Self> {
        let (master_fd, slave_fd) = open_pty(size)?;

        let child_pid = unsafe { libc::fork() };
        if child_pid < 0 {
            unsafe {
                libc::close(master_fd);
                libc::close(slave_fd);
            }
            return Err(io::Error::last_os_error());
        }

        if child_pid == 0 {
            child_exec(master_fd, slave_fd, command, working_directory)
        }

        unsafe {
            libc::close(slave_fd);
        }
        let reader_fd = dup_fd(master_fd)?;
        set_cloexec(reader_fd)?;

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || read_loop(reader_fd, tx));

        Ok(Self {
            master_fd,
            reader_fd,
            child_pid,
            rx,
            reaped: false,
        })
    }

    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        write_all_fd(self.master_fd, bytes)
    }

    pub fn resize(&self, size: PtySize) -> io::Result<()> {
        let winsize = libc::winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: size.width_px,
            ws_ypixel: size.height_px,
        };
        let rc = unsafe { libc::ioctl(self.master_fd, libc::TIOCSWINSZ, &winsize) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn try_read(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while let Ok(chunk) = self.rx.try_recv() {
            out.push(chunk);
        }
        out
    }

    pub fn child_pid(&self) -> libc::pid_t { self.child_pid }

    pub fn master_fd(&self) -> RawFd { self.master_fd }

    pub fn try_wait(&mut self) -> io::Result<bool> {
        if self.reaped {
            return Ok(true);
        }

        let mut status = 0;
        let rc = unsafe { libc::waitpid(self.child_pid, &mut status, libc::WNOHANG) };
        if rc == 0 {
            return Ok(false);
        }
        if rc == self.child_pid {
            self.reaped = true;
            return Ok(true);
        }
        if rc < 0 {
            let err = io::Error::last_os_error();
            if matches!(err.raw_os_error(), Some(libc::ECHILD)) {
                self.reaped = true;
                return Ok(true);
            }
            return Err(err);
        }

        Ok(false)
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.reader_fd);
            libc::close(self.master_fd);
            if !self.reaped {
                libc::kill(self.child_pid, libc::SIGHUP);
                libc::waitpid(self.child_pid, std::ptr::null_mut(), libc::WNOHANG);
            }
        }
    }
}

fn open_pty(size: PtySize) -> io::Result<(RawFd, RawFd)> {
    let mut master_fd = -1;
    let mut slave_fd = -1;
    let mut winsize = libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: size.width_px,
        ws_ypixel: size.height_px,
    };

    let rc = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut winsize,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }

    enable_iutf8(master_fd)?;
    set_cloexec(master_fd)?;

    Ok((master_fd, slave_fd))
}

fn enable_iutf8(fd: RawFd) -> io::Result<()> {
    let mut attrs = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut attrs) } != 0 {
        return Err(io::Error::last_os_error());
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        attrs.c_iflag |= libc::IUTF8;
    }
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &attrs) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn dup_fd(fd: RawFd) -> io::Result<RawFd> {
    let new_fd = unsafe { libc::dup(fd) };
    if new_fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(new_fd)
    }
}

fn write_all_fd(fd: RawFd, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = unsafe { libc::write(fd, bytes.as_ptr() as *const _, bytes.len()) };
        if written < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        bytes = &bytes[written as usize..];
    }
    Ok(())
}

fn child_exec(
    master_fd: RawFd,
    slave_fd: RawFd,
    command: Option<&CStr>,
    working_directory: Option<&Path>,
) -> ! {
    unsafe {
        libc::close(master_fd);
        if libc::setsid() < 0 {
            libc::_exit(1);
        }
        if libc::ioctl(slave_fd, libc::TIOCSCTTY, 0) < 0 {
            libc::_exit(1);
        }
        if libc::dup2(slave_fd, libc::STDIN_FILENO) < 0
            || libc::dup2(slave_fd, libc::STDOUT_FILENO) < 0
            || libc::dup2(slave_fd, libc::STDERR_FILENO) < 0
        {
            libc::_exit(1);
        }
        if slave_fd > libc::STDERR_FILENO {
            libc::close(slave_fd);
        }
    }

    if let Some(dir) = working_directory {
        let dir = CString::new(dir.as_os_str().as_bytes()).unwrap_or_else(|_| CString::new("/").unwrap());
        unsafe {
            libc::chdir(dir.as_ptr());
        }
    }

    // A terminal emulator must set its own terminal identity inside the PTY.
    // Inheriting TERM from the parent terminal (for example Kitty) causes
    // shells and TUIs inside Boo to emit escape sequences for the wrong
    // terminal type.
    set_env("TERM", "xterm-256color");
    set_env("COLORTERM", "truecolor");

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let shell_c = CString::new(shell).unwrap_or_else(|_| CString::new("/bin/sh").unwrap());
    let shell_name = shell_c
        .as_c_str()
        .to_bytes()
        .rsplit(|b| *b == b'/')
        .next()
        .map(|s| CString::new(s).unwrap())
        .unwrap_or_else(|| CString::new("sh").unwrap());

    match command {
        Some(cmd) => {
            let dash_c = CString::new("-c").unwrap();
            let argv = [
                shell_name.as_ptr(),
                dash_c.as_ptr(),
                cmd.as_ptr(),
                std::ptr::null(),
            ];
            unsafe {
                libc::execvp(shell_c.as_ptr(), argv.as_ptr());
                libc::_exit(127);
            }
        }
        None => {
            let dash_login = CString::new(format!("-{}", shell_name.to_string_lossy())).unwrap();
            let argv = [dash_login.as_ptr(), std::ptr::null()];
            unsafe {
                libc::execvp(shell_c.as_ptr(), argv.as_ptr());
                libc::_exit(127);
            }
        }
    }
}

fn set_env(key: &str, value: &str) {
    let key = CString::new(key).expect("environment key should not contain NUL");
    let value = CString::new(value).expect("environment value should not contain NUL");
    unsafe {
        libc::setenv(key.as_ptr(), value.as_ptr(), 1);
    }
}

fn read_loop(fd: RawFd, tx: mpsc::Sender<Vec<u8>>) {
    let mut buf = vec![0u8; 8192];
    loop {
        let rc = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if rc == 0 {
            break;
        }
        if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }

        if tx.send(buf[..rc as usize].to_vec()).is_err() {
            break;
        }
    }
    unsafe {
        libc::close(fd);
    }
}
