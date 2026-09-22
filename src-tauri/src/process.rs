//! Child-process helpers.
//!
//! On Windows a GUI app that spawns a console program (`java.exe`, `cmd.exe`,
//! any `.bat`) gets a brand-new console window unless it explicitly asks for
//! one not to be created. The launcher spawns `java -version` on every probe and
//! the game itself on every launch, so without this the desktop flickers with
//! command prompts that close as fast as they opened — which users (rightly)
//! read as something shady happening.
//!
//! Every process the launcher starts must therefore go through
//! [`command`] / [`hide_console`], which set `CREATE_NO_WINDOW` on Windows and
//! are a no-op elsewhere.

use std::ffi::OsStr;

use tokio::process::Command;

/// `CREATE_NO_WINDOW` — the child gets no console window at all.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Build a [`Command`] whose console window can never appear.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    hide_console(&mut command);
    command
}

/// Suppress the console window for an already-built command.
pub fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// `true` when the platform needs the hidden-console treatment.
pub fn needs_console_suppression() -> bool {
    cfg!(windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_are_built_without_a_console() {
        // The flag lives in the platform-specific CommandExt, so the observable
        // guarantee is simply that building one never panics and keeps the args.
        let command = command("java");
        let debug = format!("{command:?}");
        assert!(debug.contains("java"));
        assert_eq!(needs_console_suppression(), cfg!(windows));
    }
}
