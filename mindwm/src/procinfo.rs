//! What the compositor can learn about a client from `/proc`: today, whether
//! a window belongs to a Wine process, so the shell can badge it as a Windows
//! application.
//!
//! Wine runs every Windows program under one of its loaders (`wine`,
//! `wine64`, `wine-preloader`, `wine64-preloader`, Proton ships the same
//! names) and exports `WINELOADER` into every process it starts, so either
//! the executable link or the environment gives it away. Answers are cached
//! per pid: a snapshot is built on every window change and the answer for a
//! process never changes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

thread_local! {
    static WINE_BY_PID: RefCell<HashMap<u32, bool>> = RefCell::new(HashMap::new());
}

/// Is `pid` a Wine process (a Windows program running under Wine or Proton)?
/// `None` and unreadable processes are not.
pub fn is_wine(pid: Option<u32>) -> bool {
    let Some(pid) = pid else { return false };
    if pid == 0 {
        return false;
    }
    WINE_BY_PID.with(|cache| {
        if let Some(&known) = cache.borrow().get(&pid) {
            return known;
        }
        let answer = probe_wine(Path::new("/proc").join(pid.to_string()).as_path());
        let mut cache = cache.borrow_mut();
        // Pids get recycled; keep the cache small rather than exact.
        if cache.len() >= 4096 {
            cache.clear();
        }
        cache.insert(pid, answer);
        answer
    })
}

fn probe_wine(proc_dir: &Path) -> bool {
    if let Ok(exe) = std::fs::read_link(proc_dir.join("exe")) {
        if exe_is_wine(&exe) {
            return true;
        }
    }
    match std::fs::read(proc_dir.join("environ")) {
        Ok(environ) => environ_is_wine(&environ),
        Err(_) => false,
    }
}

/// `/proc/<pid>/exe` of a Wine process is one of Wine's loaders.
pub fn exe_is_wine(exe: &Path) -> bool {
    let Some(name) = exe.file_name().and_then(|n| n.to_str()) else { return false };
    // "wine (deleted)" after an upgrade still counts.
    let name = name.split(' ').next().unwrap_or(name);
    matches!(
        name,
        "wine" | "wine64" | "wine-preloader" | "wine64-preloader" | "wineserver"
    )
}

/// Every process Wine starts carries `WINELOADER=` in its environment.
pub fn environ_is_wine(environ: &[u8]) -> bool {
    environ
        .split(|&b| b == 0)
        .any(|entry| entry.starts_with(b"WINELOADER="))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_wine_loaders() {
        assert!(exe_is_wine(Path::new("/usr/bin/wine")));
        assert!(exe_is_wine(Path::new("/usr/bin/wine64-preloader")));
        assert!(exe_is_wine(Path::new("/home/u/.steam/compatibilitytools.d/GE-Proton/files/bin/wine-preloader")));
        assert!(exe_is_wine(Path::new("/usr/bin/wine (deleted)")));
        assert!(!exe_is_wine(Path::new("/usr/bin/winetricks")));
        assert!(!exe_is_wine(Path::new("/usr/bin/firefox")));
        assert!(!exe_is_wine(Path::new("")));
    }

    #[test]
    fn recognises_wine_environment() {
        assert!(environ_is_wine(b"HOME=/home/u\0WINELOADER=/usr/bin/wine\0DISPLAY=:0\0"));
        assert!(environ_is_wine(b"WINELOADER=/opt/wine/bin/wine64\0"));
        assert!(!environ_is_wine(b"HOME=/home/u\0NOTWINELOADER=x\0WINEPREFIXY=1\0"));
        assert!(!environ_is_wine(b""));
    }

    #[test]
    fn unknown_pids_are_not_wine() {
        assert!(!is_wine(None));
        assert!(!is_wine(Some(0)));
        // A pid that cannot exist on Linux (above the default pid_max) reads as not Wine.
        assert!(!is_wine(Some(u32::MAX)));
    }
}
