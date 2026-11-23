use eframe::egui::Key;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, MapVirtualKeyExW, ToUnicodeEx, HKL, MAPVK_VSC_TO_VK_EX, VK_SHIFT,
};

/// Convert an egui physical punctuation/digit key back to its PC scan code.
/// egui represents these positions using the names from a US keyboard.
fn scan_code(key: Key) -> Option<u32> {
    use Key::*;
    let number_row = [
        Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0, Minus, Equals,
    ];
    if let Some(i) = number_row.iter().position(|&k| k == key) {
        return Some(0x02 + i as u32);
    }
    Some(match key {
        OpenBracket => 0x1a,
        CloseBracket => 0x1b,
        Semicolon => 0x27,
        Quote => 0x28,
        Backtick => 0x29,
        Backslash => 0x2b,
        Comma => 0x33,
        Period => 0x34,
        Slash => 0x35,
        _ => return None,
    })
}

pub(super) fn symbol(key: Key, shift: bool) -> Option<char> {
    // Zero selects the layout of the current UI thread.
    translated(key, shift, unsafe { GetKeyboardLayout(0) })
}

fn translated(key: Key, shift: bool, layout: HKL) -> Option<char> {
    let scan = scan_code(key)?;
    let mut state = [0u8; 256];
    if shift {
        state[VK_SHIFT as usize] = 0x80;
    }
    let mut buffer = [0u16; 8];
    // Translate only this shortcut's physical key. Flag 4 leaves the OS
    // dead-key composition buffer unchanged. A negative count reports a dead key.
    let count = unsafe {
        let vk = MapVirtualKeyExW(scan, MAPVK_VSC_TO_VK_EX, layout);
        ToUnicodeEx(
            vk,
            scan,
            state.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as i32,
            4,
            layout,
        )
    };
    if count.abs() != 1 {
        return None;
    }
    char::from_u32(u32::from(buffer[0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutList;

    #[test]
    fn installed_layouts_resolve_their_own_boolean_symbols() {
        let mut layouts = [std::ptr::null_mut(); 64];
        // Enumerate existing layouts without installing or switching any of them.
        let count = unsafe { GetKeyboardLayoutList(layouts.len() as i32, layouts.as_mut_ptr()) };
        assert!(count > 0);
        for &layout in &layouts[..count as usize] {
            // Check the standard layouts only; variant layouts have their own mappings.
            let expected = match layout as usize as u32 {
                0x040b040b | 0x0000040b => [
                    (Key::Backslash, '*'),
                    (Key::CloseBracket, '^'),
                    (Key::Num7, '/'),
                ],
                0x04090409 | 0x00000409 => [(Key::Num8, '*'), (Key::Num6, '^'), (Key::Equals, '+')],
                _ => continue,
            };
            for (key, symbol) in expected {
                assert_eq!(translated(key, true, layout), Some(symbol));
            }
        }
    }
}
