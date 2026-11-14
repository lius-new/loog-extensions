//! What differs between the packages built from this source: how the channel to the
//! Agent starts and what its targets are called. Everything else is shared.

#[cfg(transport = "ssh")]
mod names {
    pub const ID: &str = "loog.remote.ssh";
    /// The package's name in the command palette, status bar and messages.
    pub const TITLE: &str = "Remote SSH";
    /// The group of targets that connected before.
    pub const SAVED: &str = "Saved Hosts";
}
#[cfg(transport = "wsl")]
mod names {
    pub const ID: &str = "loog.remote.wsl";
    pub const TITLE: &str = "Remote WSL";
    pub const SAVED: &str = "Saved Distributions";
}
#[cfg(transport = "docker")]
mod names {
    pub const ID: &str = "loog.remote.docker";
    pub const TITLE: &str = "Remote Containers";
    pub const SAVED: &str = "Saved Containers";
}
pub use names::{ID, SAVED, TITLE};

/// The words of the palette for choosing another target.
#[cfg(transport = "ssh")]
pub const NEW: &str = "Connect to New Host…";
#[cfg(transport = "wsl")]
pub const NEW: &str = "Connect to Another Distribution…";
#[cfg(transport = "docker")]
pub const NEW: &str = "Connect to Another Container…";
/// The words of the palette for choosing another folder of the current connection.
pub const OPEN_FOLDER: &str = "Open Folder…";
/// What the field asks for while the list of targets is shown.
#[cfg(not(transport = "ssh"))]
pub const PICK: &str = if cfg!(transport = "wsl") { "Select a distribution" } else { "Select a container" };
/// What is said when the list is empty.
#[cfg(not(transport = "ssh"))]
pub const NONE_FOUND: &str = if cfg!(transport = "wsl") { "No WSL distribution was found" } else { "No running container was found" };
/// The program that lists the targets, named for a message when it is missing.
#[cfg(not(transport = "ssh"))]
pub const LISTER: &str = if cfg!(transport = "wsl") { "wsl.exe" } else { "docker" };

/// Whether `target` is something the channel can be asked for without it being read as an option.
pub fn target_valid(target: &str) -> bool {
    if target.is_empty() || target.len() > 512 || target.starts_with('-') {
        return false;
    }
    #[cfg(transport = "ssh")]
    { target.bytes().all(|c| c.is_ascii_alphanumeric() || b".@:_-[]".contains(&c)) }
    // A distribution name is what `wsl -l` prints: letters, digits and a few marks.
    #[cfg(transport = "wsl")]
    { target.bytes().all(|c| c.is_ascii_alphanumeric() || b" ._-()".contains(&c)) }
    // A container name or identifier.
    #[cfg(transport = "docker")]
    { target.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)) }
}

/// The message for a target that is not acceptable.
pub fn target_hint() -> &'static str {
    #[cfg(transport = "ssh")]
    { "Enter an SSH alias or user@host" }
    #[cfg(transport = "wsl")]
    { "Choose a WSL distribution" }
    #[cfg(transport = "docker")]
    { "Choose a container" }
}
