//! Hardware controls. One sleeping worker serializes device commands and repeats;
//! the compositor only sends key transitions and receives display feedback.
use std::{io::Read, os::{fd::OwnedFd, unix::{net::UnixStream, process::CommandExt}}, process::{Command, Stdio},
          sync::mpsc, thread, time::{Duration, Instant}};
use smithay::reexports::calloop::channel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action { VolumeUp, VolumeDown, Mute, MicMute, BrightnessUp, BrightnessDown,
                  PlayPause, Pause, Stop, Next, Previous }

impl Action {
    fn repeats(self) -> bool {
        matches!(self, Self::VolumeUp | Self::VolumeDown | Self::BrightnessUp | Self::BrightnessDown)
    }
    fn label(self) -> &'static str {
        match self {
            Self::VolumeUp | Self::VolumeDown | Self::Mute => "Volume",
            Self::MicMute => "Microphone",
            Self::BrightnessUp | Self::BrightnessDown => "Brightness",
            _ => "Playback",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Feedback {
    pub label: &'static str,
    pub value: String,
    pub level: Option<f64>,
    pub muted: bool,
    pub output: Option<String>,
}

enum Input { Press { key: u32, action: Action, output: Option<String> }, Release(u32) }

#[derive(Debug)]
pub struct MediaKeys {
    tx: mpsc::Sender<Input>,
    held: Option<u32>,
}

impl MediaKeys {
    pub fn start(events: channel::Sender<Feedback>) -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new().name("mindwm-media".into()).spawn(move || {
            let mut repeat: Option<(u32, Action, Option<String>, Instant)> = None;
            loop {
                let input = match &repeat {
                    Some((_, _, _, deadline)) => rx.recv_timeout(deadline.saturating_duration_since(Instant::now())),
                    None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                };
                let (key, action, output, delay) = match input {
                    Ok(Input::Press { key, action, output }) => (key, action, output, Duration::from_millis(400)),
                    Ok(Input::Release(key)) => {
                        if repeat.as_ref().is_some_and(|r| r.0 == key) { repeat = None; }
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let Some((key, action, output, _)) = &repeat else { continue };
                        (*key, *action, output.clone(), Duration::from_millis(80))
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let result = apply(action, run);
                let success = result.is_ok();
                let mut feedback = result.unwrap_or_else(|error| {
                    tracing::warn!(?action, %error, "media control unavailable");
                    Feedback { label: action.label(), value: "Unavailable".into(), level: None, muted: true, output: None }
                });
                feedback.output = output.clone();
                if events.send(feedback).is_err() { break; }
                repeat = (success && action.repeats()).then(|| (key, action, output, Instant::now() + delay));
            }
        }).expect("start media control worker");
        Self { tx, held: None }
    }
    pub fn press(&mut self, key: u32, action: Action, output: Option<String>) {
        self.held = Some(key);
        let _ = self.tx.send(Input::Press { key, action, output });
    }
    pub fn release(&mut self, key: u32) {
        if self.held == Some(key) {
            self.held = None;
            let _ = self.tx.send(Input::Release(key));
        }
    }
    pub fn stop(&mut self) { if let Some(key) = self.held { self.release(key); } }
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    run_timeout(program, args, Duration::from_secs(2))
}

fn run_timeout(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    // Drain output while the command runs; neither a full pipe nor a helper
    // inheriting stdout may bypass the deadline or consume unbounded memory.
    let (mut reader, writer) = UnixStream::pair().map_err(|e| e.to_string())?;
    reader.set_nonblocking(true).map_err(|e| e.to_string())?;
    let mut child = Command::new(program).args(args).env("LC_ALL", "C")
        .stdin(Stdio::null()).stdout(Stdio::from(OwnedFd::from(writer))).stderr(Stdio::null())
        .process_group(0).spawn().map_err(|e| format!("{program}: {e}"))?;
    let deadline = Instant::now() + timeout;
    let result = (|| {
        let mut output = Vec::new();
        let mut eof = false;
        loop {
            let mut buffer = [0; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => { eof = true; break; }
                    Ok(count) => {
                        output.extend_from_slice(&buffer[..count]);
                        if output.len() > 16384 { return Err(format!("{program}: excessive output")); }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(format!("{program}: {e}")),
                }
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if !status.success() { return Err(format!("{program}: {status}")); }
                if eof { return String::from_utf8(output).map_err(|e| e.to_string()); }
            }
            if Instant::now() >= deadline { return Err(format!("{program}: timed out")); }
            thread::sleep(Duration::from_millis(10));
        }
    })();
    if result.is_err() {
        unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL); }
    }
    let _ = child.wait();
    result
}

fn parse_volume(output: &str) -> Result<(f64, bool), String> {
    let mut fields = output.split_whitespace();
    if fields.next() != Some("Volume:") { return Err("Invalid audio response".into()); }
    let value = fields.next().and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0).ok_or("Invalid audio volume")?;
    Ok((value, output.contains("[MUTED]")))
}

fn apply(action: Action, mut run: impl FnMut(&str, &[&str]) -> Result<String, String>) -> Result<Feedback, String> {
    let mut result = Feedback { label: action.label(), value: String::new(), level: None, muted: false, output: None };
    match action {
        Action::VolumeUp | Action::VolumeDown | Action::Mute | Action::MicMute => {
            let target = if action == Action::MicMute { "@DEFAULT_AUDIO_SOURCE@" } else { "@DEFAULT_AUDIO_SINK@" };
            match action {
                Action::VolumeUp | Action::VolumeDown => {
                    let step = if action == Action::VolumeUp { "5%+" } else { "5%-" };
                    run("wpctl", &["set-volume", "--limit", "1.0", target, step])?;
                    run("wpctl", &["set-mute", target, "0"])?;
                }
                _ => { run("wpctl", &["set-mute", target, "toggle"])?; }
            }
            let (volume, muted) = parse_volume(&run("wpctl", &["get-volume", target])?)?;
            result.level = Some(if muted { 0.0 } else { volume.min(1.0) });
            result.muted = muted;
            result.value = if muted { "Muted".into() }
                           else if action == Action::MicMute { "On".into() }
                           else { format!("{:.0}%", volume * 100.0) };
        }
        Action::BrightnessUp | Action::BrightnessDown => {
            let step = if action == Action::BrightnessUp { "+5%" } else { "5%-" };
            let output = run("brightnessctl", &["--class=backlight", "--min-value=1", "--machine-readable", "set", step])?;
            let percent = output.trim().split(',').nth(3).and_then(|s| s.strip_suffix('%'))
                .and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                .ok_or("Invalid brightness response")?;
            result.level = Some(percent / 100.0);
            result.value = format!("{percent:.0}%");
        }
        action => {
            let (command, label) = match action {
                Action::PlayPause => ("play-pause", "Play / pause"), Action::Pause => ("pause", "Paused"),
                Action::Stop => ("stop", "Stopped"), Action::Next => ("next", "Next track"),
                Action::Previous => ("previous", "Previous track"), _ => unreachable!(),
            };
            run("playerctl", &[command])?;
            result.value = label.into();
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_steps_are_bounded_and_microphone_is_separate() {
        let mut calls = Vec::new();
        let feedback = apply(Action::VolumeUp, |program, args| {
            calls.push((program.to_string(), args.iter().map(|s| s.to_string()).collect::<Vec<_>>()));
            Ok("Volume: 1.00".into())
        }).unwrap();
        assert_eq!(calls[0].1, ["set-volume", "--limit", "1.0", "@DEFAULT_AUDIO_SINK@", "5%+"]);
        assert_eq!(calls[1].1.last().unwrap(), "0");
        assert_eq!(feedback.value, "100%");
        let feedback = apply(Action::MicMute, |_, args| {
            assert_eq!(args[1], "@DEFAULT_AUDIO_SOURCE@");
            Ok("Volume: 0.75 [MUTED]".into())
        }).unwrap();
        assert_eq!(feedback.value, "Muted");
        assert_eq!(feedback.level, Some(0.0));
    }
    #[test]
    fn invalid_feedback_and_command_failures_do_not_look_successful() {
        for text in ["", "Volume: NaN", "Volume: -0.2", "unrelated 0.5"] {
            assert!(parse_volume(text).is_err());
        }
        assert!(apply(Action::VolumeDown, |_, _| Err("No sink".into())).is_err());
        assert!(apply(Action::BrightnessDown, |_, _| Ok("bad".into())).is_err());
    }
    #[test]
    fn brightness_targets_backlights_and_never_zeroes_them() {
        let result = apply(Action::BrightnessDown, |program, args| {
            assert_eq!(program, "brightnessctl");
            assert_eq!(args, ["--class=backlight", "--min-value=1", "--machine-readable", "set", "5%-"]);
            Ok("intel_backlight,backlight,50,5%,1000\n".into())
        }).unwrap();
        assert_eq!(result.level, Some(0.05));
        assert!(!Action::PlayPause.repeats());
        assert!(!Action::MicMute.repeats());
    }
    #[test]
    fn timeout_kills_helpers_and_returns_promptly() {
        let start = Instant::now();
        assert!(run_timeout("/bin/sh", &["-c", "sleep 10"], Duration::from_millis(80)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(run_timeout("/bin/sh", &["-c", "sleep 10 & exit 0"], Duration::from_millis(80)).is_err());
        assert!(run_timeout("/bin/sh", &["-c", "printf '%20000s' x"], Duration::from_secs(1)).is_err());
    }
}
