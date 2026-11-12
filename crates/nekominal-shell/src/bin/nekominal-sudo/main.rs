//! `nekominal-sudo`: runs a command as administrator in the terminal it is typed in (Windows only; `sudo` does it elsewhere).

#[cfg(windows)]
mod windows;

fn main() {
    #[cfg(windows)]
    windows::main();
    #[cfg(not(windows))]
    {
        eprintln!("nekominal-sudo runs commands as administrator on Windows; use sudo here");
        std::process::exit(1);
    }
}
