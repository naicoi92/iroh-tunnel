//! Pure, OS-independent helpers for Win32 FFI calls, shared by the Windows
//! adapters.

/// Size of a fixed Win32 structure, as the `u32` length the API takes.
#[expect(
    clippy::cast_possible_truncation,
    reason = "invariant: fixed Win32 structures are far smaller than u32::MAX bytes"
)]
pub(crate) const fn win32_size_of<T>() -> u32 {
    std::mem::size_of::<T>() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_size_of_returns_the_byte_size_of_the_type() {
        assert_eq!(win32_size_of::<u8>(), 1);
        assert_eq!(win32_size_of::<u64>(), 8);
        assert_eq!(win32_size_of::<[u16; 3]>(), 6);
    }
}
