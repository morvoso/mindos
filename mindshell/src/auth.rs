//! Checking the user's password, for the lock screen.
//!
//! PAM, through `/etc/pam.d/mindos-lock` (which includes `system-auth`, so
//! whatever the machine is set up to accept — a password, a fingerprint
//! reader, a smart card — works here too). The shell is unprivileged; PAM's
//! own `unix_chkpwd` helper does the part that needs to read `/etc/shadow`,
//! and it only lets a process check the account it is already running as.
//! That is exactly what a lock screen needs, and nothing more.
//!
//! `pam_authenticate` blocks (a wrong password costs a second or two), so
//! every call runs on its own thread — see `App::unlock` in `app.rs`.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

/// The PAM service the lock screen authenticates against.
const SERVICE: &str = "mindos-lock";

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_CONV_ERR: c_int = 19;
const PAM_DISALLOW_NULL_AUTHTOK: c_int = 0x0001;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

#[repr(C)]
struct PamConv {
    conv: Option<
        unsafe extern "C" fn(
            num_msg: c_int,
            msg: *mut *const PamMessage,
            resp: *mut *mut PamResponse,
            appdata_ptr: *mut c_void,
        ) -> c_int,
    >,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        handle: *mut *mut c_void,
    ) -> c_int;
    fn pam_authenticate(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
    fn pam_strerror(handle: *mut c_void, error: c_int) -> *const c_char;
}

/// The password, handed to the conversation function through `appdata_ptr`.
struct Secret {
    password: CString,
}

/// Answer every prompt with the one password we were given; informational and
/// error messages are ignored (the lock screen shows its own).
unsafe extern "C" fn converse(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    if num_msg <= 0 || msg.is_null() || resp.is_null() || appdata.is_null() {
        return PAM_CONV_ERR;
    }
    let secret = &*(appdata as *const Secret);
    let count = num_msg as usize;
    // PAM frees this array itself, so it has to come from the C allocator.
    let answers = libc::calloc(count, std::mem::size_of::<PamResponse>()) as *mut PamResponse;
    if answers.is_null() {
        return PAM_CONV_ERR;
    }
    for i in 0..count {
        let message = *msg.add(i);
        let style = if message.is_null() { 0 } else { (*message).msg_style };
        let answer = &mut *answers.add(i);
        answer.resp_retcode = 0;
        answer.resp = if style == PAM_PROMPT_ECHO_OFF || style == PAM_PROMPT_ECHO_ON {
            libc::strdup(secret.password.as_ptr())
        } else {
            std::ptr::null_mut()
        };
    }
    *resp = answers;
    PAM_SUCCESS
}

/// `Ok(())` when `password` is `user`'s and the account is in good standing.
/// The error is PAM's own message, which is what the lock screen shows.
pub fn check_password(user: &str, password: &str) -> Result<(), String> {
    let service = CString::new(SERVICE).map_err(|_| "bad service name".to_string())?;
    let user_c = CString::new(user).map_err(|_| "bad user name".to_string())?;
    let secret = Secret {
        password: CString::new(password).map_err(|_| "the password contains a NUL byte".to_string())?,
    };
    let conv = PamConv {
        conv: Some(converse),
        appdata_ptr: &secret as *const Secret as *mut c_void,
    };
    let mut handle: *mut c_void = std::ptr::null_mut();
    // Safety: every pointer lives until `pam_end`, and `secret` outlives the
    // conversation because this function does not return before it.
    unsafe {
        let status = pam_start(service.as_ptr(), user_c.as_ptr(), &conv, &mut handle);
        if status != PAM_SUCCESS || handle.is_null() {
            return Err("the authentication service is unavailable".into());
        }
        let mut status = pam_authenticate(handle, PAM_DISALLOW_NULL_AUTHTOK);
        if status == PAM_SUCCESS {
            status = pam_acct_mgmt(handle, PAM_DISALLOW_NULL_AUTHTOK);
        }
        let message = if status == PAM_SUCCESS {
            None
        } else {
            let text = pam_strerror(handle, status);
            Some(if text.is_null() {
                "That password did not work.".to_string()
            } else {
                CStr::from_ptr(text).to_string_lossy().into_owned()
            })
        };
        pam_end(handle, status);
        match message {
            None => Ok(()),
            Some(text) if text.trim().is_empty() => Err("That password did not work.".into()),
            Some(text) => Err(text),
        }
    }
}

/// Who is logged in here: the account the lock screen unlocks.
pub fn current_user() -> String {
    // The environment first (it is what the session was started with), then
    // the password database, so the name is right even under `su`.
    if let Ok(name) = std::env::var("USER") {
        if !name.is_empty() {
            return name;
        }
    }
    let uid = unsafe { libc::getuid() };
    let mut passwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0i8; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let ok = unsafe {
        libc::getpwuid_r(uid, &mut passwd, buf.as_mut_ptr(), buf.len(), &mut result) == 0
            && !result.is_null()
    };
    if ok && !passwd.pw_name.is_null() {
        return unsafe { CStr::from_ptr(passwd.pw_name) }.to_string_lossy().into_owned();
    }
    String::new()
}

/// The account's full name and avatar, for the lock screen's card.
pub fn user_info() -> serde_json::Value {
    let name = current_user();
    let mut display = name.clone();
    if let Ok(text) = std::fs::read_to_string("/etc/passwd") {
        for line in text.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            if fields.len() >= 5 && fields[0] == name {
                let gecos = fields[4].split(',').next().unwrap_or("").trim();
                if !gecos.is_empty() {
                    display = gecos.to_string();
                }
                break;
            }
        }
    }
    let avatar = std::path::Path::new("/var/lib/AccountsService/icons").join(&name);
    let avatar = avatar.is_file().then(|| crate::scheme::file_url(&avatar));
    serde_json::json!({ "name": name, "display": display, "avatar": avatar })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_has_a_name() {
        // In a build container USER may be unset but the uid always resolves.
        let name = current_user();
        assert!(!name.is_empty() || unsafe { libc::getuid() } == u32::MAX);
    }

    #[test]
    fn an_empty_password_is_refused_without_reaching_pam() {
        // A NUL byte is the only thing check_password rejects before PAM.
        assert!(check_password("nobody", "pass\0word").is_err());
    }
}
