use anyhow::Result;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use signal_hook::consts::TERM_SIGNALS;
use signal_hook::iterator::Signals;
use std::os::unix::process::CommandExt;

pub fn run(cmd: &[String]) -> Result<()> {
    let shell_cmd = format!("exec {}", cmd.join(" "));

    let mut child = unsafe {
        std::process::Command::new("/bin/sh")
            .args(["-lc", &shell_cmd])
            .pre_exec(|| {
                nix::unistd::setsid().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::Other, "setsid failed")
                })?;
                Ok(())
            })
            .spawn()?
    };

    let child_pid = Pid::from_raw(child.id() as i32);
    let mut signals = Signals::new(TERM_SIGNALS)?;

    std::thread::spawn(move || {
        for raw in &mut signals {
            if let Ok(sig) = Signal::try_from(raw) {
                let _ = kill(child_pid, sig);
            }
        }
    });

    let status = child.wait()?;
    std::process::exit(status.code().unwrap_or(1));
}
