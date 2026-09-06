//! Pipes and FIFOs.

use super::vfs::{File, FileOps, Inode, O_NONBLOCK, POLLERR, POLLHUP, POLLIN, POLLOUT};
use crate::mm::errno::*;
use crate::sched::wait::WaitQueue;
use crate::sync::SpinLock;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

pub const PIPE_BUF_SIZE: usize = 64 * 1024;

pub struct Pipe {
    buf: SpinLock<PipeBuf>,
    pub readers: AtomicUsize,
    pub writers: AtomicUsize,
    pub wq: WaitQueue,
}

struct PipeBuf {
    data: Vec<u8>,
    head: usize,
    len: usize,
}

impl Pipe {
    pub fn new() -> Arc<Pipe> {
        Arc::new(Pipe {
            buf: SpinLock::new(PipeBuf { data: alloc::vec![0u8; PIPE_BUF_SIZE], head: 0, len: 0 }),
            readers: AtomicUsize::new(0),
            writers: AtomicUsize::new(0),
            wq: WaitQueue::new(),
        })
    }

    fn available(&self) -> usize {
        self.buf.lock().len
    }
    fn space(&self) -> usize {
        PIPE_BUF_SIZE - self.buf.lock().len
    }

    fn read_some(&self, out: &mut [u8]) -> usize {
        let mut b = self.buf.lock();
        let n = out.len().min(b.len);
        for i in 0..n {
            out[i] = b.data[(b.head + i) % PIPE_BUF_SIZE];
        }
        b.head = (b.head + n) % PIPE_BUF_SIZE;
        b.len -= n;
        n
    }
    fn write_some(&self, data: &[u8]) -> usize {
        let mut b = self.buf.lock();
        let n = data.len().min(PIPE_BUF_SIZE - b.len);
        for i in 0..n {
            let idx = (b.head + b.len + i) % PIPE_BUF_SIZE;
            b.data[idx] = data[i];
        }
        b.len += n;
        n
    }

    pub fn read(&self, file: &File, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            let n = self.read_some(buf);
            if n > 0 {
                self.wq.wake_all();
                return Ok(n);
            }
            if self.writers.load(Ordering::Acquire) == 0 {
                return Ok(0);
            }
            if file.flags() & O_NONBLOCK != 0 {
                return Err(EAGAIN);
            }
            self.wq.wait_until(|| self.available() > 0 || self.writers.load(Ordering::Acquire) == 0).map_err(|_| EINTR)?;
        }
    }

    pub fn write(&self, file: &File, buf: &[u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut done = 0;
        while done < buf.len() {
            if self.readers.load(Ordering::Acquire) == 0 {
                crate::proc::signal::send_to_current(13); // SIGPIPE
                return if done > 0 { Ok(done) } else { Err(EPIPE) };
            }
            // POSIX: writes of <= PIPE_BUF (4096) bytes are atomic
            let need = if buf.len() <= 4096 { buf.len() } else { 1 };
            if self.space() >= need {
                let n = self.write_some(&buf[done..]);
                done += n;
                self.wq.wake_all();
                continue;
            }
            if file.flags() & O_NONBLOCK != 0 {
                return if done > 0 { Ok(done) } else { Err(EAGAIN) };
            }
            if self.wq.wait_until(|| self.space() >= need || self.readers.load(Ordering::Acquire) == 0).is_err() {
                return if done > 0 { Ok(done) } else { Err(EINTR) };
            }
        }
        Ok(done)
    }

    pub fn poll_read(&self) -> u32 {
        let mut m = 0;
        if self.available() > 0 {
            m |= POLLIN;
        }
        if self.writers.load(Ordering::Acquire) == 0 {
            m |= POLLHUP;
        }
        m
    }
    pub fn poll_write(&self) -> u32 {
        let mut m = 0;
        if self.space() > 0 {
            m |= POLLOUT;
        }
        if self.readers.load(Ordering::Acquire) == 0 {
            m |= POLLERR;
        }
        m
    }
}

pub struct PipeReader(pub Arc<Pipe>);
pub struct PipeWriter(pub Arc<Pipe>);

impl FileOps for PipeReader {
    fn read(&self, file: &File, buf: &mut [u8]) -> Result<usize> {
        self.0.read(file, buf)
    }
    fn write(&self, _file: &File, _buf: &[u8]) -> Result<usize> {
        Err(EBADF)
    }
    fn poll(&self, _file: &File) -> u32 {
        self.0.poll_read()
    }
    fn poll_wait(&self) -> Option<&WaitQueue> {
        Some(&self.0.wq)
    }
    fn ioctl(&self, _file: &File, cmd: u32, arg: usize) -> Result<usize> {
        if cmd == 0x541B {
            crate::mm::user::write_user::<i32>(arg, self.0.available() as i32)?;
            return Ok(0);
        }
        Err(ENOTTY)
    }
    fn release(&self, _file: &File) {
        self.0.readers.fetch_sub(1, Ordering::AcqRel);
        self.0.wq.wake_all();
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

impl FileOps for PipeWriter {
    fn read(&self, _file: &File, _buf: &mut [u8]) -> Result<usize> {
        Err(EBADF)
    }
    fn write(&self, file: &File, buf: &[u8]) -> Result<usize> {
        self.0.write(file, buf)
    }
    fn poll(&self, _file: &File) -> u32 {
        self.0.poll_write()
    }
    fn poll_wait(&self) -> Option<&WaitQueue> {
        Some(&self.0.wq)
    }
    fn release(&self, _file: &File) {
        self.0.writers.fetch_sub(1, Ordering::AcqRel);
        self.0.wq.wake_all();
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// Create a pipe; returns (read end, write end).
pub fn create(flags: u32) -> (Arc<File>, Arc<File>) {
    let p = Pipe::new();
    p.readers.store(1, Ordering::Release);
    p.writers.store(1, Ordering::Release);
    let fl = flags & O_NONBLOCK;
    let r = File::new(None, Arc::new(PipeReader(p.clone())), super::vfs::O_RDONLY | fl, "pipe:");
    let w = File::new(None, Arc::new(PipeWriter(p)), super::vfs::O_WRONLY | fl, "pipe:");
    (r, w)
}

/// Both ends in one file (FIFO opened O_RDWR, or socketpair-like uses).
pub struct PipeDuplex(pub Arc<Pipe>);
impl FileOps for PipeDuplex {
    fn read(&self, file: &File, buf: &mut [u8]) -> Result<usize> {
        self.0.read(file, buf)
    }
    fn write(&self, file: &File, buf: &[u8]) -> Result<usize> {
        self.0.write(file, buf)
    }
    fn poll(&self, _file: &File) -> u32 {
        self.0.poll_read() | self.0.poll_write()
    }
    fn poll_wait(&self) -> Option<&WaitQueue> {
        Some(&self.0.wq)
    }
    fn release(&self, _file: &File) {
        self.0.readers.fetch_sub(1, Ordering::AcqRel);
        self.0.writers.fetch_sub(1, Ordering::AcqRel);
        self.0.wq.wake_all();
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// Open a named pipe. The Pipe object lives in the inode's attachment.
pub fn open_fifo(inode: Arc<super::tmpfs::TmpInode>, flags: u32) -> Result<Arc<dyn FileOps>> {
    let pipe: Arc<Pipe> = {
        let mut att = inode.attachment.lock();
        match att.as_ref().and_then(|a| a.clone().downcast::<Pipe>().ok()) {
            Some(p) => p,
            None => {
                let p = Pipe::new();
                *att = Some(p.clone());
                p
            }
        }
    };
    let acc = flags & super::vfs::O_ACCMODE;
    let nonblock = flags & O_NONBLOCK != 0;
    match acc {
        super::vfs::O_RDONLY => {
            pipe.readers.fetch_add(1, Ordering::AcqRel);
            pipe.wq.wake_all();
            if !nonblock && pipe.writers.load(Ordering::Acquire) == 0 {
                if pipe.wq.wait_until(|| pipe.writers.load(Ordering::Acquire) > 0).is_err() {
                    pipe.readers.fetch_sub(1, Ordering::AcqRel);
                    return Err(EINTR);
                }
            }
            Ok(Arc::new(PipeReader(pipe)))
        }
        super::vfs::O_WRONLY => {
            if nonblock && pipe.readers.load(Ordering::Acquire) == 0 {
                return Err(ENXIO);
            }
            pipe.writers.fetch_add(1, Ordering::AcqRel);
            pipe.wq.wake_all();
            if !nonblock && pipe.readers.load(Ordering::Acquire) == 0 {
                if pipe.wq.wait_until(|| pipe.readers.load(Ordering::Acquire) > 0).is_err() {
                    pipe.writers.fetch_sub(1, Ordering::AcqRel);
                    return Err(EINTR);
                }
            }
            Ok(Arc::new(PipeWriter(pipe)))
        }
        _ => {
            pipe.readers.fetch_add(1, Ordering::AcqRel);
            pipe.writers.fetch_add(1, Ordering::AcqRel);
            pipe.wq.wake_all();
            Ok(Arc::new(PipeDuplex(pipe)))
        }
    }
}

impl Inode for Pipe {
    fn kind(&self) -> super::vfs::Kind {
        super::vfs::Kind::Fifo
    }
    fn ino(&self) -> u64 {
        0
    }
    fn fs_id(&self) -> u64 {
        0
    }
    fn fs_name(&self) -> &'static str {
        "pipefs"
    }
    fn stat(&self) -> super::vfs::Stat {
        super::vfs::Stat::simple(0, 0, super::vfs::Kind::Fifo, 0o600, self.available() as u64, 1)
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}
