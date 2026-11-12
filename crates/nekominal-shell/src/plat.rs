//! What the application and its helper threads say to a window, which each platform delivers its own way
//! (`PostMessageW` on Windows, an event of the loop on Linux): the window handles them from its message loop, outside
//! the borrow of the application.

/// A request to a window. Sent from any thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// Something changed that the window should show (output of a pane, a finished read): see `App::on_wake`.
    Wake,
    /// The graphics device was lost; `forced` makes everything again on a device that still works.
    GpuLost { forced: bool },
    /// Close the window as its close button does.
    Close,
    /// Destroy the window now, without asking.
    DestroyWindow,
    NewWindow,
    /// Show, hide or make the drop-down window.
    Quake,
    /// Open the agents panel with the panes of every window.
    Agents,
    /// Show a pane (its id) of the tab with this uid.
    ShowPane { tab: u64, pane: u32 },
    /// Hand every window to the updater.
    HandOff,
    /// The elevated copy answered a request (its id).
    Elevated(u64),
    /// A command of the menu of the window: minimize, maximize, restore, move, size or close.
    Window(WindowCommand),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowCommand {
    Minimize,
    Maximize,
    Restore,
    Move,
    Size,
    Close,
}
