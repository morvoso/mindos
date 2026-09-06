//! Linux errno values and the kernel `Result` type.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Errno(pub i32);

pub type Result<T> = core::result::Result<T, Errno>;

impl Errno {
    pub const fn as_ret(self) -> u64 {
        (-(self.0 as i64)) as u64
    }
    pub fn name(self) -> &'static str {
        match self.0 {
            1 => "EPERM",
            2 => "ENOENT",
            3 => "ESRCH",
            4 => "EINTR",
            5 => "EIO",
            6 => "ENXIO",
            7 => "E2BIG",
            8 => "ENOEXEC",
            9 => "EBADF",
            10 => "ECHILD",
            11 => "EAGAIN",
            12 => "ENOMEM",
            13 => "EACCES",
            14 => "EFAULT",
            16 => "EBUSY",
            17 => "EEXIST",
            18 => "EXDEV",
            19 => "ENODEV",
            20 => "ENOTDIR",
            21 => "EISDIR",
            22 => "EINVAL",
            23 => "ENFILE",
            24 => "EMFILE",
            25 => "ENOTTY",
            27 => "EFBIG",
            28 => "ENOSPC",
            29 => "ESPIPE",
            30 => "EROFS",
            31 => "EMLINK",
            32 => "EPIPE",
            34 => "ERANGE",
            35 => "EDEADLK",
            36 => "ENAMETOOLONG",
            38 => "ENOSYS",
            39 => "ENOTEMPTY",
            40 => "ELOOP",
            75 => "EOVERFLOW",
            88 => "ENOTSOCK",
            95 => "EOPNOTSUPP",
            97 => "EAFNOSUPPORT",
            98 => "EADDRINUSE",
            107 => "ENOTCONN",
            110 => "ETIMEDOUT",
            111 => "ECONNREFUSED",
            _ => "E?",
        }
    }
}

macro_rules! errnos {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: Errno = Errno($v);)* };
}
errnos! {
    EPERM = 1, ENOENT = 2, ESRCH = 3, EINTR = 4, EIO = 5, ENXIO = 6, E2BIG = 7, ENOEXEC = 8,
    EBADF = 9, ECHILD = 10, EAGAIN = 11, ENOMEM = 12, EACCES = 13, EFAULT = 14, EBUSY = 16,
    EEXIST = 17, EXDEV = 18, ENODEV = 19, ENOTDIR = 20, EISDIR = 21, EINVAL = 22, ENFILE = 23,
    EMFILE = 24, ENOTTY = 25, EFBIG = 27, ENOSPC = 28, ESPIPE = 29, EROFS = 30, EMLINK = 31,
    EPIPE = 32, EDOM = 33, ERANGE = 34, EDEADLK = 35, ENAMETOOLONG = 36, ENOLCK = 37, ENOSYS = 38,
    ENOTEMPTY = 39, ELOOP = 40, ENOMSG = 42, EOVERFLOW = 75, ENOTSOCK = 88, EDESTADDRREQ = 89,
    EMSGSIZE = 90, EPROTOTYPE = 91, ENOPROTOOPT = 92, EPROTONOSUPPORT = 93, EOPNOTSUPP = 95,
    EAFNOSUPPORT = 97, EADDRINUSE = 98, EADDRNOTAVAIL = 99, ENETUNREACH = 101, ECONNABORTED = 103,
    ECONNRESET = 104, ENOBUFS = 105, EISCONN = 106, ENOTCONN = 107, ETIMEDOUT = 110,
    ECONNREFUSED = 111, EALREADY = 114, EINPROGRESS = 115, ERESTARTSYS = 512,
}
