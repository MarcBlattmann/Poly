//! The sessions of the machine: `WTSGetActiveConsoleSessionId` (kernel32) and
//! `WTSEnumerateSessionsW` (wtsapi32) (`specs/windows/kernel32.dll/WTSGetActiveConsoleSessionId.yaml`,
//! `specs/windows/wtsapi32.dll/WTSEnumerateSessionsW.yaml`).
//!
//! Poly's machine is one with a user at the console, as the oracle was: session 0 is the
//! services' session ("Services", disconnected) and the user's desktop is the console session,
//! 1 ("Console", active). A process in another session (one started with a token of another
//! session) is listed too. Constants: `wtsapi32.h` (`WTS_CONNECTSTATE_CLASS`), `winerror.h`.

use alloc::vec::Vec;

/// The console session (`WTSGetActiveConsoleSessionId`): the desktop of the user at the console.
pub const CONSOLE_SESSION: u32 = 1;
/// `WTSActive` and `WTSDisconnected` (`WTS_CONNECTSTATE_CLASS`).
pub const WTS_ACTIVE: u32 = 0;
pub const WTS_DISCONNECTED: u32 = 4;
/// `ERROR_INVALID_USER_BUFFER` (`winerror.h`).
pub const ERROR_INVALID_USER_BUFFER: u32 = 1784;

/// One `WTS_SESSION_INFOW`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEntry {
    pub id: u32,
    pub name: &'static str,
    pub state: u32,
}

/// The sessions, ascending: 0 ("Services", disconnected), the console (active), and the caller's
/// own session when it is neither (active: it has a process, so someone is logged on there).
pub fn sessions(own: u32) -> Vec<SessionEntry> {
    let mut v = alloc::vec![
        SessionEntry { id: 0, name: "Services", state: WTS_DISCONNECTED },
        SessionEntry { id: CONSOLE_SESSION, name: "Console", state: WTS_ACTIVE },
    ];
    if own != 0 && own != CONSOLE_SESSION {
        v.push(SessionEntry { id: own, name: "Session", state: WTS_ACTIVE });
        v.sort_by_key(|e| e.id);
    }
    v
}

/// The size of one `WTS_SESSION_INFOW` (`DWORD SessionId; LPWSTR pWinStationName;
/// WTS_CONNECTSTATE_CLASS State`): 24 bytes with 8-byte pointers, 12 with 4-byte ones.
pub fn entry_size(pointer: usize) -> usize {
    if pointer == 8 { 24 } else { 12 }
}

/// The block `WTSEnumerateSessionsW` returns, for an allocation at `base`: the array, then the
/// names (NUL-terminated UTF-16), each entry's name pointer into the same block.
pub fn session_block(entries: &[SessionEntry], base: u64, pointer: usize) -> Vec<u8> {
    let size = entry_size(pointer);
    let mut out = alloc::vec![0u8; size * entries.len()];
    for (i, e) in entries.iter().enumerate() {
        let name_at = base + out.len() as u64;
        for c in e.name.encode_utf16().chain(core::iter::once(0)) {
            out.extend_from_slice(&c.to_le_bytes());
        }
        let at = i * size;
        out[at..at + 4].copy_from_slice(&e.id.to_le_bytes());
        if pointer == 8 {
            out[at + 8..at + 16].copy_from_slice(&name_at.to_le_bytes());
            out[at + 16..at + 20].copy_from_slice(&e.state.to_le_bytes());
        } else {
            out[at + 4..at + 8].copy_from_slice(&(name_at as u32).to_le_bytes());
            out[at + 8..at + 12].copy_from_slice(&e.state.to_le_bytes());
        }
    }
    out
}

/// The bytes [`session_block`] makes for `entries` (to allocate them first).
pub fn block_size(entries: &[SessionEntry], pointer: usize) -> usize {
    entries.iter().map(|e| entry_size(pointer) + 2 * (e.name.encode_utf16().count() + 1)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_console_machine() {
        let s = sessions(1);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].id, s[0].name, s[0].state), (0, "Services", 4));
        assert_eq!((s[1].id, s[1].name, s[1].state), (1, "Console", 0));
        assert_eq!(sessions(0), s);
        let s = sessions(3);
        assert_eq!(s.iter().map(|e| e.id).collect::<Vec<_>>(), [0, 1, 3]);
    }

    #[test]
    fn the_block_layout() {
        let s = sessions(1);
        let b = session_block(&s, 0x1000, 8);
        assert_eq!(b.len(), block_size(&s, 8));
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 1);
        let name = u64::from_le_bytes(b[32..40].try_into().unwrap()) - 0x1000;
        assert_eq!(name as usize, 48 + 2 * 9, "after the array and \"Services\\0\"");
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 0);
        let n = name as usize;
        let text: Vec<u16> = b[n..n + 14].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(alloc::string::String::from_utf16(&text).unwrap(), "Console");
        let b = session_block(&s, 0x1000, 4);
        assert_eq!(b.len(), block_size(&s, 4));
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 0x1000 + 24);
        assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 4);
    }
}
