//! Child-process spawn policy: never flash a console window on Windows.

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Chain on every spawned `Command` so children never open a console window.
pub trait NoWindow {
    fn no_window(&mut self) -> &mut Self;
}

impl NoWindow for tokio::process::Command {
    #[cfg(windows)]
    fn no_window(&mut self) -> &mut Self {
        self.creation_flags(CREATE_NO_WINDOW);
        self
    }

    #[cfg(not(windows))]
    fn no_window(&mut self) -> &mut Self {
        self
    }
}

impl NoWindow for std::process::Command {
    #[cfg(windows)]
    fn no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(CREATE_NO_WINDOW);
        self
    }

    #[cfg(not(windows))]
    fn no_window(&mut self) -> &mut Self {
        self
    }
}
