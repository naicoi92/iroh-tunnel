//! Pure decoding of one raw ACE (as returned by `GetAce`) into the fields the
//! trust model needs. No `unsafe`: the Win32 adapter hands over the ACE bytes
//! and turns the SID at [`AceLayout::sid_offset`] into its string form.
//!
//! Mapping (conservative: a refusal is preferred over a missed grant):
//! - `ACCESS_ALLOWED_ACE_TYPE` (0) is an allow ACE.
//! - `ACCESS_DENIED_ACE_TYPE` (1) is a deny ACE (the model ignores it).
//! - Every other ACE type that carries a SID (object, callback, audit, label,
//!   …) is treated as an allow ACE with its mask, so a grant hidden in an
//!   unusual ACE type is never missed.
//! - An ACE whose SID cannot be located inside `AceSize` yields `None`; the
//!   adapter refuses that path as having no usable access list.

/// The decoded fields of one ACE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AceLayout {
    pub allow: bool,
    pub mask: u32,
    pub inherit_only: bool,
    /// Byte offset of the SID from the start of the ACE.
    pub sid_offset: usize,
    /// Byte length of that SID (`8 + 4 * SubAuthorityCount`).
    pub sid_len: usize,
}

const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const INHERIT_ONLY_ACE: u8 = 0x8;
/// `ACE_OBJECT_TYPE_PRESENT` / `ACE_INHERITED_OBJECT_TYPE_PRESENT`.
const OBJECT_TYPE_PRESENT: u32 = 0x1;
const INHERITED_OBJECT_TYPE_PRESENT: u32 = 0x2;
const HEADER_AND_MASK: usize = 8;
const GUID_LEN: usize = 16;
const SID_FIXED_LEN: usize = 8;

/// `ACCESS_*_OBJECT_ACE` layouts (allowed/denied/audit/alarm object and their
/// callback variants): a `Flags` field and up to two GUIDs precede the SID.
fn is_object_type(ace_type: u8) -> bool {
    matches!(ace_type, 0x5..=0x8 | 0xB | 0xC | 0xF | 0x10)
}

/// Types whose SID starts right after the mask (`SidStart` at offset 8).
fn is_plain_type(ace_type: u8) -> bool {
    matches!(ace_type, 0x0..=0x3 | 0x9 | 0xA | 0xD | 0xE | 0x11..=0x15)
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let raw = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(raw.try_into().ok()?))
}

/// Decode `bytes`, the ACE starting at its header. `None` when the ACE is
/// truncated, of an unknown layout (compound ACE), or its SID would run past
/// `AceSize`.
pub(super) fn ace_layout(bytes: &[u8]) -> Option<AceLayout> {
    let (&ace_type, &flags) = (bytes.first()?, bytes.get(1)?);
    let size = usize::from(u16::from_le_bytes([*bytes.get(2)?, *bytes.get(3)?]));
    if size < HEADER_AND_MASK {
        return None;
    }
    let ace = bytes.get(..size)?;
    let mask = u32_at(ace, 4)?;
    let sid_offset = if is_plain_type(ace_type) {
        HEADER_AND_MASK
    } else if is_object_type(ace_type) {
        let object_flags = u32_at(ace, HEADER_AND_MASK)?;
        let guids = [OBJECT_TYPE_PRESENT, INHERITED_OBJECT_TYPE_PRESENT]
            .iter()
            .filter(|bit| object_flags & *bit != 0)
            .count();
        HEADER_AND_MASK + 4 + GUID_LEN * guids
    } else {
        return None;
    };
    let sub_authorities = usize::from(*ace.get(sid_offset + 1)?);
    let sid_len = SID_FIXED_LEN + 4 * sub_authorities;
    if sid_offset + sid_len > size {
        return None;
    }
    Some(AceLayout {
        allow: ace_type != ACCESS_DENIED_ACE_TYPE,
        mask,
        inherit_only: flags & INHERIT_ONLY_ACE != 0,
        sid_offset,
        sid_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SID S-1-5-21-7-1001 as raw bytes (revision, count, authority BE, subs LE).
    fn sid() -> Vec<u8> {
        let mut v = vec![1, 3, 0, 0, 0, 0, 0, 5];
        for sub in [21u32, 7, 1001] {
            v.extend_from_slice(&sub.to_le_bytes());
        }
        v
    }

    fn ace(ace_type: u8, flags: u8, mask: u32, body: &[u8]) -> Vec<u8> {
        let size = (HEADER_AND_MASK + body.len()) as u16;
        let mut v = vec![ace_type, flags];
        v.extend_from_slice(&size.to_le_bytes());
        v.extend_from_slice(&mask.to_le_bytes());
        v.extend_from_slice(body);
        v
    }

    fn object_body(object_flags: u32, guids: usize) -> Vec<u8> {
        let mut v = object_flags.to_le_bytes().to_vec();
        v.extend(std::iter::repeat_n(0xAB, GUID_LEN * guids));
        v.extend(sid());
        v
    }

    fn layout(allow: bool, mask: u32, inherit_only: bool, sid_offset: usize) -> AceLayout {
        AceLayout {
            allow,
            mask,
            inherit_only,
            sid_offset,
            sid_len: 20,
        }
    }

    #[test]
    fn plain_allow_and_deny() {
        assert_eq!(
            ace_layout(&ace(0, 0, 0x1F_01FF, &sid())),
            Some(layout(true, 0x1F_01FF, false, 8))
        );
        assert_eq!(
            ace_layout(&ace(1, 0, 0x2, &sid())),
            Some(layout(false, 0x2, false, 8))
        );
    }

    #[test]
    fn callback_and_audit_aces_count_as_allow() {
        // ACCESS_DENIED_CALLBACK (0xA) too: conservative, never a hidden grant.
        for t in [0x2u8, 0x9, 0xA, 0x11] {
            assert_eq!(
                ace_layout(&ace(t, 0, 0x4, &sid())),
                Some(layout(true, 0x4, false, 8)),
                "type {t:#x}"
            );
        }
    }

    #[test]
    fn object_ace_skips_present_guids() {
        let cases = [
            (0u32, 0usize),
            (OBJECT_TYPE_PRESENT, 1),
            (INHERITED_OBJECT_TYPE_PRESENT, 1),
            (OBJECT_TYPE_PRESENT | INHERITED_OBJECT_TYPE_PRESENT, 2),
        ];
        for (object_flags, guids) in cases {
            let a = ace(0x5, 0, 0x2, &object_body(object_flags, guids));
            assert_eq!(
                ace_layout(&a),
                Some(layout(true, 0x2, false, 12 + GUID_LEN * guids)),
                "{object_flags:#x}"
            );
        }
    }

    #[test]
    fn inherit_only_flag_is_read() {
        assert_eq!(
            ace_layout(&ace(0, INHERIT_ONLY_ACE | 0x3, 0x1000_0000, &sid())),
            Some(layout(true, 0x1000_0000, true, 8))
        );
        assert!(!ace_layout(&ace(0, 0x10, 0x1, &sid())).unwrap().inherit_only);
    }

    #[test]
    fn truncated_buffer_is_none() {
        let full = ace(0, 0, 0x1, &sid());
        for len in [0, 3, 7, 8, 15, full.len() - 1] {
            assert_eq!(ace_layout(&full[..len]), None, "len {len}");
        }
    }

    #[test]
    fn sid_past_ace_size_is_none() {
        let mut a = ace(0, 0, 0x1, &sid());
        // Claim 4 sub-authorities: the SID would need 24 bytes, only 20 exist.
        a[9] = 4;
        assert_eq!(ace_layout(&a), None);
        // AceSize shorter than the bytes actually carried.
        let mut b = ace(0, 0, 0x1, &sid());
        b[2..4].copy_from_slice(&16u16.to_le_bytes());
        assert_eq!(ace_layout(&b), None);
        // An object ACE whose flags announce GUIDs that are not there.
        let c = ace(0x5, 0, 0x1, &object_body(OBJECT_TYPE_PRESENT, 0));
        assert_eq!(ace_layout(&c), None);
    }

    #[test]
    fn unknown_or_compound_type_is_none() {
        for t in [0x4u8, 0x16, 0xFF] {
            assert_eq!(ace_layout(&ace(t, 0, 0x1, &sid())), None, "type {t:#x}");
        }
    }
}
