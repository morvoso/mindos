//! execve: ELF loading and initial stack construction.

use super::process::Process;
use crate::arch::x86_64::interrupts::TrapFrame;
use crate::fs::path::resolve_path;
use crate::fs::vfs::{self, Inode, Kind};
use crate::mm::addrspace::{AddressSpace, Backing, MAP_FIXED, MAP_PRIVATE, PROT_EXEC, PROT_READ, PROT_WRITE, STACK_SIZE, STACK_TOP};
use crate::mm::errno::*;
use crate::mm::user::copy_to_user;
use crate::mm::PAGE_SIZE;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;

const PT_LOAD: u32 = 1;
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
const PT_GNU_STACK: u32 = 0x6474e551;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;

const AT_NULL: u64 = 0;
const AT_PHDR: u64 = 3;
const AT_PHENT: u64 = 4;
const AT_PHNUM: u64 = 5;
const AT_PAGESZ: u64 = 6;
const AT_BASE: u64 = 7;
const AT_FLAGS: u64 = 8;
const AT_ENTRY: u64 = 9;
const AT_UID: u64 = 11;
const AT_EUID: u64 = 12;
const AT_GID: u64 = 13;
const AT_EGID: u64 = 14;
const AT_PLATFORM: u64 = 15;
const AT_HWCAP: u64 = 16;
const AT_CLKTCK: u64 = 17;
const AT_SECURE: u64 = 23;
const AT_RANDOM: u64 = 25;
const AT_HWCAP2: u64 = 26;
const AT_EXECFN: u64 = 31;
const AT_MINSIGSTKSZ: u64 = 51;

const ET_DYN_BASE: usize = 0x5555_5555_4000;
const INTERP_BASE: usize = 0x7f00_0000_0000 - 0x1000_0000;

#[derive(Clone, Copy)]
struct Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

struct ElfInfo {
    e_type: u16,
    entry: u64,
    phoff: u64,
    phentsize: u16,
    phnum: u16,
    phdrs: Vec<Phdr>,
    interp: Option<String>,
}

fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn rd64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn parse_elf(inode: &Arc<dyn Inode>) -> Result<ElfInfo> {
    let mut hdr = [0u8; 64];
    let n = inode.read_at(0, &mut hdr)?;
    if n < 64 || &hdr[0..4] != b"\x7fELF" {
        return Err(ENOEXEC);
    }
    if hdr[4] != 2 || hdr[5] != 1 {
        return Err(ENOEXEC); // not ELF64 little-endian
    }
    let e_type = rd16(&hdr, 16);
    let machine = rd16(&hdr, 18);
    if machine != 62 || (e_type != ET_EXEC && e_type != ET_DYN) {
        return Err(ENOEXEC);
    }
    let entry = rd64(&hdr, 24);
    let phoff = rd64(&hdr, 32);
    let phentsize = rd16(&hdr, 54);
    let phnum = rd16(&hdr, 56);
    if phentsize != 56 || phnum == 0 || phnum > 512 {
        return Err(ENOEXEC);
    }
    let mut buf = alloc::vec![0u8; phnum as usize * 56];
    let n = inode.read_at(phoff, &mut buf)?;
    if n < buf.len() {
        return Err(ENOEXEC);
    }
    let mut phdrs = Vec::with_capacity(phnum as usize);
    let mut interp = None;
    for i in 0..phnum as usize {
        let o = i * 56;
        let ph = Phdr {
            p_type: rd32(&buf, o),
            p_flags: rd32(&buf, o + 4),
            p_offset: rd64(&buf, o + 8),
            p_vaddr: rd64(&buf, o + 16),
            p_filesz: rd64(&buf, o + 32),
            p_memsz: rd64(&buf, o + 40),
            p_align: rd64(&buf, o + 48),
        };
        if ph.p_type == PT_INTERP {
            let mut s = alloc::vec![0u8; ph.p_filesz as usize];
            inode.read_at(ph.p_offset, &mut s)?;
            while s.last() == Some(&0) {
                s.pop();
            }
            interp = Some(String::from_utf8(s).map_err(|_| ENOEXEC)?);
        }
        phdrs.push(ph);
    }
    Ok(ElfInfo { e_type, entry, phoff, phentsize, phnum, phdrs, interp })
}

struct Loaded {
    base: usize,
    entry: usize,
    phdr_addr: usize,
    brk: usize,
}

/// Map an ELF image into `aspace`. `base` is the load bias for ET_DYN.
fn load_elf(aspace: &Arc<AddressSpace>, inode: &Arc<dyn Inode>, elf: &ElfInfo, base: usize, path: &'static str) -> Result<Loaded> {
    let mut brk = 0usize;
    let mut phdr_addr = 0usize;
    let mut first_load: Option<u64> = None;
    for ph in &elf.phdrs {
        if ph.p_type == PT_PHDR {
            phdr_addr = base + ph.p_vaddr as usize;
        }
        if ph.p_type != PT_LOAD {
            continue;
        }
        if ph.p_memsz == 0 {
            continue;
        }
        if first_load.is_none() {
            first_load = Some(ph.p_vaddr);
        }
        let mut prot = 0;
        if ph.p_flags & PF_R != 0 {
            prot |= PROT_READ;
        }
        if ph.p_flags & PF_W != 0 {
            prot |= PROT_WRITE;
        }
        if ph.p_flags & PF_X != 0 {
            prot |= PROT_EXEC;
        }
        let vaddr = base + ph.p_vaddr as usize;
        let page_off = vaddr & (PAGE_SIZE - 1);
        let map_start = vaddr - page_off;
        let file_off = ph.p_offset as usize - page_off;
        if (ph.p_offset as usize & (PAGE_SIZE - 1)) != page_off {
            return Err(ENOEXEC);
        }
        let file_end = vaddr + ph.p_filesz as usize;
        let mem_end = vaddr + ph.p_memsz as usize;
        let file_map_end = (file_end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let mem_map_end = (mem_end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        if ph.p_filesz > 0 {
            aspace.mmap(map_start, file_map_end - map_start, prot, MAP_PRIVATE | MAP_FIXED, Backing::File { inode: inode.clone(), offset: file_off as u64 }, path)?;
        }
        if mem_map_end > file_map_end {
            aspace.mmap(file_map_end, mem_map_end - file_map_end, prot, MAP_PRIVATE | MAP_FIXED, Backing::Anon, "[bss]")?;
        }
        // zero the tail of the last file-backed page
        if ph.p_memsz > ph.p_filesz && file_end & (PAGE_SIZE - 1) != 0 && prot & PROT_WRITE != 0 {
            let zero_len = file_map_end - file_end;
            let zeros = alloc::vec![0u8; zero_len];
            copy_to_user(file_end, &zeros)?;
        }
        if mem_end > brk {
            brk = mem_end;
        }
        let _ = ph.p_align;
    }
    if phdr_addr == 0 {
        // phdrs live inside the first PT_LOAD normally
        if let Some(fl) = first_load {
            phdr_addr = base + (elf.phoff - (elf.phdrs.iter().find(|p| p.p_type == PT_LOAD).map(|p| p.p_offset).unwrap_or(0))) as usize + fl as usize;
        }
    }
    Ok(Loaded { base, entry: base + elf.entry as usize, phdr_addr, brk })
}

pub struct ExecImage {
    pub inode: Arc<dyn Inode>,
    pub path: String,
    pub argv: Vec<String>,
    pub envp: Vec<String>,
}

/// Resolve `#!` scripts (up to 4 levels) to the final ELF to run.
fn resolve_script(cwd: &Arc<dyn Inode>, mut path: String, mut argv: Vec<String>) -> Result<(Arc<dyn Inode>, String, Vec<String>)> {
    for _ in 0..4 {
        let r = resolve_path(cwd, &path, true)?;
        let inode = r.inode.ok_or(ENOENT)?;
        if inode.kind() == Kind::Dir {
            return Err(EACCES);
        }
        if inode.kind() != Kind::File {
            return Err(EACCES);
        }
        let mut head = [0u8; 256];
        let n = inode.read_at(0, &mut head)?;
        if n >= 2 && &head[0..2] == b"#!" {
            let line_end = head[..n].iter().position(|&b| b == b'\n').unwrap_or(n);
            let line = core::str::from_utf8(&head[2..line_end]).map_err(|_| ENOEXEC)?.trim();
            let mut parts = line.splitn(2, |c: char| c == ' ' || c == '\t');
            let interp = parts.next().ok_or(ENOEXEC)?;
            if interp.is_empty() {
                return Err(ENOEXEC);
            }
            let arg = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());
            let mut nargv = alloc::vec![String::from(interp)];
            if let Some(a) = arg {
                nargv.push(String::from(a));
            }
            nargv.push(path.clone());
            if !argv.is_empty() {
                nargv.extend(argv.into_iter().skip(1));
            }
            argv = nargv;
            path = String::from(interp);
            continue;
        }
        return Ok((inode, path, argv));
    }
    Err(ELOOP)
}

fn leak_name(s: &str) -> &'static str {
    // VMA names are &'static; keep executable paths for the process lifetime.
    alloc::boxed::Box::leak(String::from(s).into_boxed_str())
}

/// Replace the current process image. On success the task's trap frame is
/// rewritten to start the new program; the syscall returns into it.
pub fn do_execve(frame: &mut TrapFrame, path: &str, argv: Vec<String>, envp: Vec<String>) -> Result<()> {
    let t = crate::sched::current();
    let proc = t.proc.clone();
    let (cwd, _) = proc.cwd();
    let (inode, _final_path, argv) = resolve_script(&cwd, String::from(path), argv)?;
    let elf = parse_elf(&inode)?;
    // the interpreter, if any, must be loadable before we commit
    let interp = match &elf.interp {
        Some(ip) => {
            let r = resolve_path(&cwd, ip, true)?;
            let ii = r.inode.ok_or(ENOENT)?;
            let ie = parse_elf(&ii)?;
            if ie.e_type != ET_DYN {
                return Err(ELIBBAD);
            }
            Some((ii, ie, ip.clone()))
        }
        None => None,
    };
    let abs_path = if path.starts_with('/') { crate::fs::path::normalize(path) } else { crate::fs::path::join(&proc.cwd().1, path) };

    // ---- point of no return ----
    let aspace = AddressSpace::new();
    let old = proc.set_aspace(Some(aspace.clone()));
    aspace.activate();
    drop(old);
    // other threads of a multithreaded process are terminated
    {
        let threads = proc.threads.lock().clone();
        for th in threads {
            if !Arc::ptr_eq(&th, &t) {
                th.post_signal(super::signal::SIGKILL);
            }
        }
    }
    proc.vfork_done.store(true, Ordering::Release);
    proc.vfork_wq.wake_all();

    let exe_name = leak_name(&abs_path);
    let base = if elf.e_type == ET_DYN { ET_DYN_BASE } else { 0 };
    let main = match load_elf(&aspace, &inode, &elf, base, exe_name) {
        Ok(l) => l,
        Err(e) => {
            klog!("exec", "failed to load {}: {}", abs_path, e.name());
            super::exit::exit_group_signal(super::signal::SIGSEGV);
        }
    };
    aspace.set_brk_start(main.brk);
    let mut entry = main.entry;
    let mut interp_base = 0usize;
    if let Some((ii, ie, ip)) = &interp {
        let l = match load_elf(&aspace, ii, ie, INTERP_BASE, leak_name(ip)) {
            Ok(l) => l,
            Err(e) => {
                klog!("exec", "failed to load interpreter {}: {}", ip, e.name());
                super::exit::exit_group_signal(super::signal::SIGSEGV);
            }
        };
        entry = l.entry;
        interp_base = l.base;
    }

    // stack
    let stack_lo = STACK_TOP - STACK_SIZE;
    if aspace.mmap(stack_lo, STACK_SIZE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_FIXED, Backing::Anon, "[stack]").is_err() {
        super::exit::exit_group_signal(super::signal::SIGSEGV);
    }
    let sp = match build_stack(&argv, &envp, &abs_path, &elf, &main, interp_base) {
        Ok(sp) => sp,
        Err(_) => super::exit::exit_group_signal(super::signal::SIGSEGV),
    };

    // process bookkeeping
    let files = proc.files();
    let closed = files.close_on_exec();
    drop(closed);
    proc.sighand().reset_for_exec();
    {
        let mut s = t.sig.lock();
        s.altstack_sp = 0;
        s.altstack_size = 0;
        s.altstack_flags = 2;
    }
    let comm: String = abs_path.rsplit('/').next().unwrap_or("?").chars().take(15).collect();
    *proc.comm.lock() = comm.clone();
    *t.name.lock() = comm;
    *proc.exe.lock() = abs_path.clone();
    *proc.cmdline.lock() = argv.clone();
    t.fs_base.store(0, Ordering::Relaxed);
    t.clear_child_tid.store(0, Ordering::Relaxed);
    crate::arch::x86_64::msr::write(crate::arch::x86_64::msr::IA32_FS_BASE, 0);
    // fresh FPU state
    let size = crate::arch::x86_64::cpu::features().xsave_size as usize;
    crate::arch::x86_64::cpu::init_fpu_area(unsafe { core::slice::from_raw_parts_mut(t.fpu_area(), size) });
    unsafe { crate::arch::x86_64::cpu::xrstor(t.fpu_area()) };

    *frame = TrapFrame::new_user(entry as u64, sp as u64);
    Ok(())
}

fn build_stack(argv: &[String], envp: &[String], execfn: &str, elf: &ElfInfo, main: &Loaded, interp_base: usize) -> Result<usize> {
    let mut sp = STACK_TOP;
    let mut push_bytes = |sp: &mut usize, data: &[u8]| -> Result<usize> {
        *sp -= data.len();
        copy_to_user(*sp, data)?;
        Ok(*sp)
    };
    // strings
    let execfn_addr = push_bytes(&mut sp, execfn.as_bytes().iter().copied().chain([0u8]).collect::<Vec<u8>>().as_slice())?;
    let platform_addr = push_bytes(&mut sp, b"x86_64\0")?;
    let mut random = [0u8; 16];
    crate::dev::chardev::fill_random(&mut random);
    let random_addr = push_bytes(&mut sp, &random)?;
    let mut env_addrs = Vec::with_capacity(envp.len());
    for e in envp.iter().rev() {
        let mut b = e.as_bytes().to_vec();
        b.push(0);
        env_addrs.push(push_bytes(&mut sp, &b)?);
    }
    env_addrs.reverse();
    let mut arg_addrs = Vec::with_capacity(argv.len());
    for a in argv.iter().rev() {
        let mut b = a.as_bytes().to_vec();
        b.push(0);
        arg_addrs.push(push_bytes(&mut sp, &b)?);
    }
    arg_addrs.reverse();
    sp &= !15;

    let hwcap = crate::arch::x86_64::cpu::cpuid(1, 0).edx as u64;
    let auxv: Vec<(u64, u64)> = alloc::vec![
        (AT_SYSINFO_EHDR_NONE, 0),
        (AT_HWCAP, hwcap),
        (AT_PAGESZ, PAGE_SIZE as u64),
        (AT_CLKTCK, 100),
        (AT_PHDR, main.phdr_addr as u64),
        (AT_PHENT, elf.phentsize as u64),
        (AT_PHNUM, elf.phnum as u64),
        (AT_BASE, interp_base as u64),
        (AT_FLAGS, 0),
        (AT_ENTRY, main.entry as u64),
        (AT_UID, 0),
        (AT_EUID, 0),
        (AT_GID, 0),
        (AT_EGID, 0),
        (AT_SECURE, 0),
        (AT_RANDOM, random_addr as u64),
        (AT_HWCAP2, 0),
        (AT_EXECFN, execfn_addr as u64),
        (AT_PLATFORM, platform_addr as u64),
        (AT_MINSIGSTKSZ, 4096 + crate::arch::x86_64::cpu::features().xsave_size as u64),
        (AT_NULL, 0),
    ];
    let auxv: Vec<(u64, u64)> = auxv.into_iter().filter(|(k, _)| *k != AT_SYSINFO_EHDR_NONE).collect();
    // total words: argc + argv + NULL + envp + NULL + auxv pairs
    let words = 1 + arg_addrs.len() + 1 + env_addrs.len() + 1 + auxv.len() * 2;
    sp -= words * 8;
    sp &= !15;
    let mut w = sp;
    let mut put = |w: &mut usize, v: u64| -> Result<()> {
        copy_to_user(*w, &v.to_le_bytes())?;
        *w += 8;
        Ok(())
    };
    put(&mut w, arg_addrs.len() as u64)?;
    for a in &arg_addrs {
        put(&mut w, *a as u64)?;
    }
    put(&mut w, 0)?;
    for e in &env_addrs {
        put(&mut w, *e as u64)?;
    }
    put(&mut w, 0)?;
    for (k, v) in &auxv {
        put(&mut w, *k)?;
        put(&mut w, *v)?;
    }
    Ok(sp)
}

const AT_SYSINFO_EHDR_NONE: u64 = u64::MAX;
const ELIBBAD: Errno = Errno(80);

/// Create PID 1: the calling kernel thread must be owned by the init process
/// (see `spawn_init`); it execs `path` and enters user mode.
fn init_thread(_arg: usize) {
    let (path, argv, envp) = INIT_ARGS.lock().take().expect("init args");
    let t = crate::sched::current();
    let p = t.proc.clone();
    let root = crate::fs::path::root();
    p.set_cwd(root, String::from("/"));
    // stdio on the console
    let console = crate::fs::lookup_abs("/dev/console").expect("/dev/console");
    let ops = console.open(vfs::O_RDWR).expect("open console").expect("console ops");
    let f = vfs::File::new(Some(console), ops, vfs::O_RDWR, "/dev/console");
    let files = p.files();
    files.alloc(f.clone(), false, 0).unwrap();
    files.alloc(f.clone(), false, 0).unwrap();
    files.alloc(f, false, 0).unwrap();
    crate::proc::process::set_controlling_tty(crate::dev::tty::console(), true);
    let frame = t.user_frame();
    match do_execve(frame, &path, argv, envp) {
        Ok(()) => {
            let fp = t.user_frame_ptr();
            drop(p);
            drop(t);
            unsafe { crate::arch::x86_64::interrupts::enter_user_frame(fp) }
        }
        Err(e) => panic!("cannot exec init {}: {}", path, e.name()),
    }
}

static INIT_ARGS: crate::sync::SpinLock<Option<(String, Vec<String>, Vec<String>)>> = crate::sync::SpinLock::new(None);

/// Create the init process (pid 1) running `path`.
pub fn spawn_init(path: &str, argv: Vec<String>, envp: Vec<String>) {
    let p = Process::new();
    assert_eq!(p.pid, 1, "init must be pid 1");
    *p.comm.lock() = String::from("init");
    *INIT_ARGS.lock() = Some((String::from(path), argv, envp));
    let task = crate::sched::task::Task::new_kernel_in(p.clone(), "init", init_thread, 0);
    p.add_thread(task.clone());
    crate::sched::enqueue(task);
}
