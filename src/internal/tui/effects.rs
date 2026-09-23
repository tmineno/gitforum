//! Side effects of TUI input handling that reach outside the process:
//! the system clipboard and the terminal's mouse-capture mode.
//!
//! Input handlers call these through `App::effects` so tests can swap in
//! [`RecordingEffects`] and never touch the developer's clipboard or
//! terminal (doc/spec/TUI-UX-TESTING.md, ADR-012 decision 2).

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;

pub(crate) trait TuiEffects {
    /// Put `text` on the system clipboard.
    fn set_clipboard(&mut self, text: &str) -> std::io::Result<()>;
    /// Turn terminal mouse capture on or off. Errors are ignored: a
    /// terminal that refuses the mode change keeps working as before.
    fn set_mouse_capture(&mut self, enabled: bool);
}

/// The real effects, used by `git forum tui`.
pub(crate) struct TerminalEffects;

impl TuiEffects for TerminalEffects {
    fn set_clipboard(&mut self, text: &str) -> std::io::Result<()> {
        copy_to_clipboard(text)
    }

    fn set_mouse_capture(&mut self, enabled: bool) {
        if enabled {
            execute!(std::io::stdout(), EnableMouseCapture).ok();
        } else {
            execute!(std::io::stdout(), DisableMouseCapture).ok();
        }
    }
}

/// Copy text to the system clipboard.
///
/// Tries platform-specific commands in order:
/// - macOS: `pbcopy`
/// - Linux/Wayland: `wl-copy`
/// - Linux/X11: `xclip -selection clipboard`
/// - Linux/X11 fallback: `xsel --clipboard --input`
///
/// Returns `Ok(())` on success or an error if no clipboard tool is available.
fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};

    let candidates: &[&[&str]] = &[
        &["pbcopy"],
        &["wl-copy"],
        &["xclip", "-selection", "clipboard"],
        &["xsel", "--clipboard", "--input"],
    ];

    for args in candidates {
        let program = args[0];
        let extra = &args[1..];
        if let Ok(mut child) = Command::new(program)
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            use std::io::Write;
            // Write data and drop stdin so the child sees EOF
            let write_ok = match child.stdin.take() {
                Some(mut stdin) => {
                    let res = stdin.write_all(text.as_bytes());
                    drop(stdin);
                    res.is_ok()
                }
                None => false,
            };
            if write_ok {
                if let Ok(status) = child.wait() {
                    if status.success() {
                        return Ok(());
                    }
                }
            } else {
                child.kill().ok();
                child.wait().ok();
            }
            // This candidate failed; try the next one
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no clipboard tool found (install pbcopy, wl-copy, xclip, or xsel)",
    ))
}

/// One recorded call on [`RecordingEffects`].
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EffectCall {
    Clipboard(String),
    MouseCapture(bool),
}

/// Test effects: record every call and touch nothing. Clones share one log,
/// so a test keeps a clone and reads `calls` after handing the other to `App`.
#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct RecordingEffects {
    pub(crate) calls: std::rc::Rc<std::cell::RefCell<Vec<EffectCall>>>,
}

#[cfg(test)]
impl TuiEffects for RecordingEffects {
    fn set_clipboard(&mut self, text: &str) -> std::io::Result<()> {
        self.calls
            .borrow_mut()
            .push(EffectCall::Clipboard(text.to_string()));
        Ok(())
    }

    fn set_mouse_capture(&mut self, enabled: bool) {
        self.calls
            .borrow_mut()
            .push(EffectCall::MouseCapture(enabled));
    }
}
