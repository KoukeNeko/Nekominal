//! Usage: winprobe <logfile> <mode> [setup-bytes-as-hex]
//!
//! mode `win`   : classic console input (ReadConsoleInputW, no VT input), like cmd.exe or
//!                PSReadLine; mouse and window events enabled.
//! mode `vt`    : ENABLE_VIRTUAL_TERMINAL_INPUT, like programs that parse VT input themselves.
//!
//! Every record is logged, one line each; `S <hex>` lines carry the UTF-8 of the characters
//! the key-down records delivered in one read (the VT byte stream in `vt` mode).

use std::{fs::OpenOptions, io::Write};

use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, ReadConsoleInputW, SetConsoleMode, ENABLE_EXTENDED_FLAGS,
    ENABLE_MOUSE_INPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_WINDOW_INPUT, FOCUS_EVENT,
    INPUT_RECORD, KEY_EVENT, MOUSE_EVENT, STD_INPUT_HANDLE, WINDOW_BUFFER_SIZE_EVENT,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let log_path = &args[1];
    let mode_arg = args[2].as_str();
    let quiet = mode_arg.ends_with("-quiet");
    let mode = mode_arg.trim_end_matches("-quiet");
    let setup = args.get(3).map(|hex| {
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect::<Vec<u8>>()
    });

    let mut log = OpenOptions::new().create(true).append(true).open(log_path).unwrap();
    let mut line = |text: String| {
        writeln!(log, "{text}").unwrap();
        log.flush().unwrap();
    };

    // SAFETY: plain Win32 calls on the process's own stdin handle.
    unsafe {
        let input = GetStdHandle(STD_INPUT_HANDLE);
        let mut original = 0u32;
        GetConsoleMode(input, &mut original);
        let wanted = match mode {
            "vt" => ENABLE_WINDOW_INPUT | ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_EXTENDED_FLAGS,
            _ => ENABLE_WINDOW_INPUT | ENABLE_MOUSE_INPUT | ENABLE_EXTENDED_FLAGS,
        };
        let set = SetConsoleMode(input, wanted);
        let mut now = 0u32;
        GetConsoleMode(input, &mut now);
        line(format!("INFO mode={mode} original={original:#06x} wanted={wanted:#06x} set={set} now={now:#06x}"));

        if let Some(setup) = setup {
            let mut out = std::io::stdout();
            out.write_all(&setup).unwrap();
            out.flush().unwrap();
        }
        println!("READY");
        std::io::stdout().flush().unwrap();

        let mut records: [INPUT_RECORD; 256] = std::mem::zeroed();
        // A surrogate pair can straddle two reads; keep a lone high surrogate for the next one.
        let mut carry: Vec<u16> = Vec::new();
        loop {
            let mut count = 0u32;
            if ReadConsoleInputW(input, records.as_mut_ptr(), records.len() as u32, &mut count) == 0 {
                line("ERROR ReadConsoleInputW failed".to_string());
                return;
            }
            let mut units: Vec<u16> = std::mem::take(&mut carry);
            for record in &records[..count as usize] {
                match record.EventType as u32 {
                    KEY_EVENT => {
                        let key = record.Event.KeyEvent;
                        let unit = key.uChar.UnicodeChar;
                        if !quiet {
                        line(format!(
                            "K down={} vk={:#04x} sc={:#04x} uc={:#06x} cs={:#05x} rc={}",
                            key.bKeyDown, key.wVirtualKeyCode, key.wVirtualScanCode, unit, key.dwControlKeyState, key.wRepeatCount
                        ));
                        }
                        if key.bKeyDown != 0 && unit != 0 {
                            units.push(unit);
                        }
                    }
                    MOUSE_EVENT => {
                        let mouse = record.Event.MouseEvent;
                        line(format!(
                            "M x={} y={} buttons={:#x} cs={:#05x} flags={:#x}",
                            mouse.dwMousePosition.X, mouse.dwMousePosition.Y, mouse.dwButtonState, mouse.dwControlKeyState, mouse.dwEventFlags
                        ));
                    }
                    FOCUS_EVENT => line(format!("F focus={}", record.Event.FocusEvent.bSetFocus)),
                    WINDOW_BUFFER_SIZE_EVENT => {
                        let size = record.Event.WindowBufferSizeEvent.dwSize;
                        line(format!("W cols={} rows={}", size.X, size.Y));
                    }
                    other => line(format!("O type={other}")),
                }
            }
            if units.last().is_some_and(|u| (0xd800..0xdc00).contains(u)) {
                carry.push(units.pop().unwrap());
            }
            if !units.is_empty() {
                let text = String::from_utf16_lossy(&units);
                let hex: String = text.bytes().map(|b| format!("{b:02x}")).collect();
                line(format!("S {hex}"));
            }
        }
    }
}
