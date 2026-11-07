//! Pure trust model: may a chain of path elements (root to target) be trusted?
//!
//! No Win32 calls. Descriptors come from an adapter; this module only decides.

use std::fmt;
use std::path::PathBuf;

use crate::error::CliError;

/// A security identifier in string form (`S-1-5-18`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Sid(pub String);

pub(crate) const SYSTEM: &str = "S-1-5-18";
pub(crate) const ADMINISTRATORS: &str = "S-1-5-32-544";
pub(crate) const TRUSTED_INSTALLER: &str =
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
pub(crate) const EVERYONE: &str = "S-1-1-0";
pub(crate) const CREATOR_OWNER: &str = "S-1-3-0";

pub(crate) const FILE_WRITE_DATA: u32 = 0x2;
pub(crate) const FILE_ADD_FILE: u32 = 0x2;
pub(crate) const FILE_APPEND_DATA: u32 = 0x4;
pub(crate) const FILE_ADD_SUBDIRECTORY: u32 = 0x4;
pub(crate) const FILE_WRITE_EA: u32 = 0x10;
pub(crate) const FILE_DELETE_CHILD: u32 = 0x40;
pub(crate) const DELETE: u32 = 0x1_0000;
pub(crate) const WRITE_DAC: u32 = 0x4_0000;
pub(crate) const WRITE_OWNER: u32 = 0x8_0000;
pub(crate) const GENERIC_ALL: u32 = 0x1000_0000;
pub(crate) const GENERIC_WRITE: u32 = 0x4000_0000;

const FILE_TARGET_MASK: u32 = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | DELETE
    | WRITE_DAC
    | WRITE_OWNER
    | GENERIC_WRITE
    | GENERIC_ALL;
/// A directory target may gain entries (add file/subdirectory/EA) but must
/// not lose or replace trusted ones.
const DIR_TARGET_MASK: u32 = (FILE_TARGET_MASK | FILE_DELETE_CHILD)
    & !(FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY | FILE_WRITE_EA);
const ANCESTOR_MASK: u32 = DELETE | FILE_DELETE_CHILD | WRITE_DAC | WRITE_OWNER | GENERIC_ALL;

/// Right names. The refused directory mask contains no bit shared with a
/// FILE_* name (0x2, 0x4, 0x10 are excluded), so the file-style names are
/// unambiguous wherever they are reported.
const RIGHT_NAMES: [(u32, &str); 9] = [
    (FILE_WRITE_DATA, "FILE_WRITE_DATA"),
    (FILE_APPEND_DATA, "FILE_APPEND_DATA"),
    (FILE_WRITE_EA, "FILE_WRITE_EA"),
    (FILE_DELETE_CHILD, "FILE_DELETE_CHILD"),
    (DELETE, "DELETE"),
    (WRITE_DAC, "WRITE_DAC"),
    (WRITE_OWNER, "WRITE_OWNER"),
    (GENERIC_ALL, "GENERIC_ALL"),
    (GENERIC_WRITE, "GENERIC_WRITE"),
];

/// Parse `^S-1-\d+(-\d+)+$`.
pub(crate) fn parse_sid(s: &str) -> Option<Sid> {
    let rest = s.strip_prefix("S-1-")?;
    let mut parts = rest.split('-');
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let first = parts.next()?;
    if !digits(first) {
        return None;
    }
    let mut more = 0;
    for p in parts {
        if !digits(p) {
            return None;
        }
        more += 1;
    }
    (more >= 1).then(|| Sid(s.to_string()))
}

#[derive(Debug, Clone)]
pub(crate) struct Ace {
    pub sid: Sid,
    pub mask: u32,
    pub inherit_only: bool,
    pub allow: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Descriptor {
    pub path: PathBuf,
    pub owner: Sid,
    /// `None` is a NULL DACL (everyone has full access).
    pub dacl: Option<Vec<Ace>>,
    pub reparse: bool,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TrustedSet(Vec<Sid>);

impl TrustedSet {
    /// Config/state files: system principals plus the pinned user.
    pub(crate) fn for_user_files(pinned: &Sid) -> Self {
        let mut set = Self::for_binary();
        set.0.push(pinned.clone());
        set
    }

    pub(crate) fn for_binary() -> Self {
        Self(vec![
            Sid(SYSTEM.into()),
            Sid(ADMINISTRATORS.into()),
            Sid(TRUSTED_INSTALLER.into()),
        ])
    }

    pub(crate) fn machine() -> Self {
        Self::for_binary()
    }

    fn contains(&self, sid: &Sid) -> bool {
        self.0.contains(sid)
    }
}

/// Largest file `read_verified` returns (1 MiB).
pub(crate) const MAX_VERIFIED_READ: usize = 1 << 20;

/// Whether reading `chunk` more bytes after `total` passes [`MAX_VERIFIED_READ`].
pub(crate) fn exceeds_read_limit(total: usize, chunk: usize) -> bool {
    total.saturating_add(chunk) > MAX_VERIFIED_READ
}

/// Byte length of a `FILE_RENAME_INFO` buffer holding `name_units` UTF-16
/// units: the fixed part up to the `FileName` offset, the name and a NUL,
/// never below the struct's own size (which already holds one unit).
pub(crate) fn rename_buffer_len(
    struct_size: usize,
    name_offset: usize,
    name_units: usize,
) -> usize {
    (name_offset + (name_units + 1) * 2).max(struct_size)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    Reparse {
        path: PathBuf,
    },
    NoDacl {
        path: PathBuf,
    },
    /// The DACL exists but holds an ACE that cannot be decoded.
    UnreadableDacl {
        path: PathBuf,
    },
    Owner {
        path: PathBuf,
        owner: Sid,
    },
    Access {
        path: PathBuf,
        sid: Sid,
        rights: Vec<&'static str>,
    },
    /// UNC, device, relative or otherwise non-local path.
    Remote {
        path: PathBuf,
    },
    /// Chain shape or path segment that cannot be verified safely
    /// (alternate data stream, wildcard, trailing dot/space, file ancestor).
    Malformed {
        path: PathBuf,
    },
    /// No elements were supplied.
    EmptyChain,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reparse { path } => {
                write!(
                    f,
                    "{} is a reparse point (symlink or junction)",
                    path.display()
                )
            }
            Self::NoDacl { path } => {
                write!(
                    f,
                    "{} has no access list (NULL DACL: everyone has access)",
                    path.display()
                )
            }
            Self::UnreadableDacl { path } => {
                write!(
                    f,
                    "{} has an access list that cannot be read",
                    path.display()
                )
            }
            Self::Owner { path, owner } => {
                write!(
                    f,
                    "{} is owned by untrusted SID {}",
                    path.display(),
                    owner.0
                )
            }
            Self::Access { path, sid, rights } => write!(
                f,
                "{} grants untrusted SID {} {}",
                path.display(),
                sid.0,
                rights.join(" | ")
            ),
            Self::Remote { path } => {
                write!(f, "{} is not a local absolute path", path.display())
            }
            Self::Malformed { path } => {
                write!(
                    f,
                    "{} is malformed or has an unsafe segment",
                    path.display()
                )
            }
            Self::EmptyChain => write!(f, "nothing to verify"),
        }
    }
}

impl std::error::Error for Refusal {}

impl From<Refusal> for CliError {
    fn from(r: Refusal) -> Self {
        CliError::Permission(r.to_string())
    }
}

/// Split an absolute local path into root → target prefixes.
///
/// Takes `&str` so the logic runs on every host. Strips `\\?\`; refuses UNC
/// (`\\server\share`, `\\?\UNC\…`), device paths, relative paths and `..`
/// with [`Refusal::Remote`]. `/` is accepted as a separator.
pub(crate) fn components(path: &str) -> Result<Vec<String>, Refusal> {
    let remote = || Refusal::Remote {
        path: PathBuf::from(path),
    };
    let norm = path.replace('/', "\\");
    let body = match norm.strip_prefix("\\\\?\\") {
        Some(rest) => rest.to_string(),
        None => norm.clone(),
    };
    let b = body.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        return Err(remote());
    }
    let root = format!("{}:\\", body[..1].to_ascii_uppercase());
    let mut out = vec![root.clone()];
    let mut acc = root;
    for seg in body[3..].split('\\') {
        match seg {
            "" | "." => continue,
            ".." => return Err(remote()),
            s => {
                if s.contains([':', '*', '?', '<', '>', '|', '"']) || s.ends_with(['.', ' ']) {
                    return Err(Refusal::Malformed {
                        path: PathBuf::from(path),
                    });
                }
                if !acc.ends_with('\\') {
                    acc.push('\\');
                }
                acc.push_str(s);
                out.push(acc.clone());
            }
        }
    }
    Ok(out)
}

fn names(mask: u32) -> Vec<&'static str> {
    RIGHT_NAMES
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, n)| *n)
        .collect()
}

/// Evaluate a chain ordered from the volume root to the target. The last
/// element is the target (file or directory mask); every earlier element is
/// an ancestor (ancestor mask) and must be a directory. Returns the first
/// offending element (chain order) and, within it, the first offending ACE
/// (DACL order). An empty chain is refused, never trusted.
pub(crate) fn evaluate(chain: &[Descriptor], trusted: &TrustedSet) -> Result<(), Refusal> {
    let Some(last) = chain.len().checked_sub(1) else {
        return Err(Refusal::EmptyChain);
    };
    for (i, d) in chain.iter().enumerate() {
        if i != last && !d.is_dir {
            return Err(Refusal::Malformed {
                path: d.path.clone(),
            });
        }
        if d.reparse {
            return Err(Refusal::Reparse {
                path: d.path.clone(),
            });
        }
        let Some(dacl) = &d.dacl else {
            return Err(Refusal::NoDacl {
                path: d.path.clone(),
            });
        };
        if !trusted.contains(&d.owner) {
            return Err(Refusal::Owner {
                path: d.path.clone(),
                owner: d.owner.clone(),
            });
        }
        let refused = match (i == last, d.is_dir) {
            (false, _) => ANCESTOR_MASK,
            (true, true) => DIR_TARGET_MASK,
            (true, false) => FILE_TARGET_MASK,
        };
        for ace in dacl {
            if !ace.allow || ace.inherit_only || trusted.contains(&ace.sid) {
                continue;
            }
            let hit = ace.mask & refused;
            if hit != 0 {
                return Err(Refusal::Access {
                    path: d.path.clone(),
                    sid: ace.sid.clone(),
                    rights: names(hit),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FA: u32 = 0x1F_01FF;
    const RX: u32 = 0x12_00A9;
    const GXGR: u32 = 0xA000_0000;
    const LC: u32 = 0x4;

    fn pinned() -> Sid {
        Sid("S-1-5-21-1-2-3-1001".into())
    }
    fn bob() -> Sid {
        Sid("S-1-5-21-1-2-3-1002".into())
    }
    fn s(x: &str) -> Sid {
        Sid(x.into())
    }
    fn allow(sid: Sid, mask: u32) -> Ace {
        Ace {
            sid,
            mask,
            inherit_only: false,
            allow: true,
        }
    }
    fn d(path: &str, owner: Sid, aces: Vec<Ace>, reparse: bool, is_dir: bool) -> Descriptor {
        Descriptor {
            path: path.into(),
            owner,
            dacl: Some(aces),
            reparse,
            is_dir,
        }
    }
    fn set() -> TrustedSet {
        TrustedSet::for_user_files(&pinned())
    }
    fn good_chain() -> Vec<Descriptor> {
        vec![
            d("C:\\", s(SYSTEM), vec![allow(s(SYSTEM), FA)], false, true),
            d("C:\\u", pinned(), vec![allow(pinned(), FA)], false, true),
            d(
                "C:\\u\\s.toml",
                pinned(),
                vec![allow(pinned(), FA)],
                false,
                false,
            ),
        ]
    }

    #[test]
    fn trusted_profile_chain_passes() {
        assert_eq!(evaluate(&good_chain(), &set()), Ok(()));
    }

    #[test]
    fn reparse_component_is_refused() {
        let mut c = good_chain();
        c[1].reparse = true;
        assert_eq!(
            evaluate(&c, &set()),
            Err(Refusal::Reparse {
                path: "C:\\u".into()
            })
        );
    }

    #[test]
    fn null_dacl_is_refused() {
        let mut c = good_chain();
        c[2].dacl = None;
        assert_eq!(
            evaluate(&c, &set()),
            Err(Refusal::NoDacl {
                path: "C:\\u\\s.toml".into()
            })
        );
    }

    #[test]
    fn null_dacl_refusal_names_the_missing_access_list() {
        let text = Refusal::NoDacl {
            path: "C:\\u\\s.toml".into(),
        }
        .to_string();
        assert!(
            text.contains("C:\\u\\s.toml") && text.contains("access list"),
            "{text}"
        );
    }

    #[test]
    fn unreadable_dacl_has_its_own_refusal_text_and_is_a_permission_error() {
        let r = Refusal::UnreadableDacl {
            path: "C:\\u\\s.toml".into(),
        };
        let text = r.to_string();
        assert_eq!(text, "C:\\u\\s.toml has an access list that cannot be read");
        assert!(!text.contains("everyone"), "{text}");
        assert!(matches!(CliError::from(r), CliError::Permission(m) if m == text));
    }

    #[test]
    fn rename_buffer_covers_name_and_nul() {
        // x64 layout: 20-byte header padded to 24 bytes, struct size 24 + 2.
        assert_eq!(rename_buffer_len(32, 20, 0), 32);
        assert_eq!(rename_buffer_len(32, 20, 5), 32);
        assert_eq!(rename_buffer_len(32, 20, 10), 42);
        assert_eq!(rename_buffer_len(32, 20, 100), 222);
    }

    #[test]
    fn read_limit_allows_exactly_one_mebibyte() {
        assert!(!exceeds_read_limit(0, MAX_VERIFIED_READ));
        assert!(!exceeds_read_limit(MAX_VERIFIED_READ - 1, 1));
        assert!(exceeds_read_limit(MAX_VERIFIED_READ, 1));
        assert!(exceeds_read_limit(usize::MAX, 1));
    }

    #[test]
    fn untrusted_owner_is_refused() {
        let mut c = good_chain();
        c[2].owner = bob();
        assert_eq!(
            evaluate(&c, &set()),
            Err(Refusal::Owner {
                path: "C:\\u\\s.toml".into(),
                owner: bob()
            })
        );
    }

    #[test]
    fn inherit_only_full_control_is_ignored() {
        let mut c = good_chain();
        c[2].dacl.as_mut().unwrap().push(Ace {
            sid: s(CREATOR_OWNER),
            mask: GENERIC_ALL,
            inherit_only: true,
            allow: true,
        });
        assert_eq!(evaluate(&c, &set()), Ok(()));
    }

    #[test]
    fn each_file_write_right_is_refused() {
        let rights = [
            (FILE_WRITE_DATA, "FILE_WRITE_DATA"),
            (FILE_APPEND_DATA, "FILE_APPEND_DATA"),
            (FILE_WRITE_EA, "FILE_WRITE_EA"),
            (DELETE, "DELETE"),
            (WRITE_DAC, "WRITE_DAC"),
            (WRITE_OWNER, "WRITE_OWNER"),
            (GENERIC_WRITE, "GENERIC_WRITE"),
            (GENERIC_ALL, "GENERIC_ALL"),
        ];
        for (bit, name) in rights {
            let mut c = good_chain();
            c[2].dacl.as_mut().unwrap().push(allow(s(EVERYONE), bit));
            match evaluate(&c, &set()) {
                Err(Refusal::Access { rights, sid, .. }) => {
                    assert!(rights.contains(&name), "{name}: {rights:?}");
                    assert_eq!(sid, s(EVERYONE));
                }
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    #[test]
    fn directory_target_refuses_delete_child_but_allows_create() {
        let dir = |mask| {
            vec![
                d("C:\\", s(SYSTEM), vec![allow(s(SYSTEM), FA)], false, true),
                d("C:\\x", pinned(), vec![allow(bob(), mask)], false, true),
            ]
        };
        assert_eq!(
            evaluate(
                &dir(FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY | FILE_WRITE_EA),
                &set()
            ),
            Ok(())
        );
        match evaluate(&dir(FILE_DELETE_CHILD), &set()) {
            Err(Refusal::Access { rights, .. }) => assert_eq!(rights, vec!["FILE_DELETE_CHILD"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn each_ancestor_replace_right_is_refused() {
        for (bit, name) in [
            (DELETE, "DELETE"),
            (FILE_DELETE_CHILD, "FILE_DELETE_CHILD"),
            (WRITE_DAC, "WRITE_DAC"),
            (WRITE_OWNER, "WRITE_OWNER"),
            (GENERIC_ALL, "GENERIC_ALL"),
        ] {
            let mut c = good_chain();
            c[1].dacl.as_mut().unwrap().push(allow(bob(), bit));
            match evaluate(&c, &set()) {
                Err(Refusal::Access { path, rights, .. }) => {
                    assert_eq!(path, PathBuf::from("C:\\u"));
                    assert!(rights.contains(&name));
                }
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    #[test]
    fn create_rights_on_ancestor_are_accepted() {
        let mut c = good_chain();
        c[0].dacl
            .as_mut()
            .unwrap()
            .push(allow(s("S-1-5-11"), FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY));
        assert_eq!(evaluate(&c, &set()), Ok(()));
    }

    #[test]
    fn write_attributes_is_accepted() {
        let mut c = good_chain();
        c[2].dacl.as_mut().unwrap().push(allow(bob(), 0x100));
        assert_eq!(evaluate(&c, &set()), Ok(()));
    }

    #[test]
    fn deny_aces_are_ignored() {
        let mut c = good_chain();
        c[2].dacl.as_mut().unwrap().push(Ace {
            sid: bob(),
            mask: GENERIC_ALL,
            inherit_only: false,
            allow: false,
        });
        assert_eq!(evaluate(&c, &set()), Ok(()));
    }

    #[test]
    fn binary_set_excludes_pinned_user() {
        assert_eq!(
            evaluate(&good_chain(), &TrustedSet::for_binary()),
            Err(Refusal::Owner {
                path: "C:\\u".into(),
                owner: pinned()
            })
        );
        assert_eq!(TrustedSet::machine().0, TrustedSet::for_binary().0);
    }

    #[test]
    fn refusal_names_path_owner_sid_and_rights() {
        let mut c = good_chain();
        c[2].dacl
            .as_mut()
            .unwrap()
            .push(allow(bob(), FILE_WRITE_DATA | DELETE));
        let r = evaluate(&c, &set()).unwrap_err();
        let text = r.to_string();
        assert!(
            text.contains("C:\\u\\s.toml") && text.contains(&bob().0),
            "{text}"
        );
        assert!(
            text.contains("FILE_WRITE_DATA") && text.contains("DELETE"),
            "{text}"
        );
        let o = Refusal::Owner {
            path: "C:\\p".into(),
            owner: bob(),
        }
        .to_string();
        assert!(o.contains("C:\\p") && o.contains(&bob().0));
        assert!(matches!(CliError::from(r), CliError::Permission(_)));
    }

    #[test]
    fn unc_path_is_refused() {
        assert!(matches!(
            components("\\\\server\\share\\x"),
            Err(Refusal::Remote { .. })
        ));
        assert!(matches!(
            components("\\\\?\\UNC\\server\\share"),
            Err(Refusal::Remote { .. })
        ));
    }

    #[test]
    fn verbatim_prefix_is_normalized_before_walk() {
        assert_eq!(
            components("\\\\?\\C:\\a").unwrap(),
            components("C:\\a").unwrap()
        );
    }

    #[test]
    fn walk_starts_at_the_config_volume_root() {
        assert_eq!(
            components("D:\\cfg\\s.toml").unwrap(),
            vec!["D:\\", "D:\\cfg", "D:\\cfg\\s.toml"]
        );
    }

    fn mal(p: &str) -> Refusal {
        Refusal::Malformed { path: p.into() }
    }

    #[test]
    fn empty_chain_is_refused() {
        assert_eq!(evaluate(&[], &set()), Err(Refusal::EmptyChain));
        assert_eq!(Refusal::EmptyChain.to_string(), "nothing to verify");
        assert!(matches!(
            CliError::from(Refusal::EmptyChain),
            CliError::Permission(_)
        ));
    }

    #[test]
    fn non_directory_ancestor_is_malformed() {
        let mut c = good_chain();
        c[1].is_dir = false;
        assert_eq!(evaluate(&c, &set()), Err(mal("C:\\u")));
    }

    #[test]
    fn untrusted_fa_is_refused_on_file_dir_and_ancestor() {
        for idx in [0usize, 1, 2] {
            let mut c = good_chain();
            c[idx].dacl.as_mut().unwrap().push(allow(bob(), FA));
            assert!(
                matches!(evaluate(&c, &set()), Err(Refusal::Access { .. })),
                "file/ancestor idx {idx}"
            );
        }
        let mut c = good_chain();
        c.pop();
        c.push(d(
            "C:\\u\\dir",
            pinned(),
            vec![allow(pinned(), FA), allow(bob(), FA)],
            false,
            true,
        ));
        assert!(matches!(evaluate(&c, &set()), Err(Refusal::Access { .. })));
    }

    #[test]
    fn untrusted_owner_on_ancestor_is_refused() {
        let mut c = good_chain();
        c[1].owner = bob();
        assert_eq!(
            evaluate(&c, &set()),
            Err(Refusal::Owner {
                path: "C:\\u".into(),
                owner: bob()
            })
        );
    }

    #[test]
    fn reparse_at_first_and_last_position() {
        for (idx, p) in [(0usize, "C:\\"), (2, "C:\\u\\s.toml")] {
            let mut c = good_chain();
            c[idx].reparse = true;
            assert_eq!(
                evaluate(&c, &set()),
                Err(Refusal::Reparse { path: p.into() })
            );
        }
    }

    #[test]
    fn first_bad_element_in_chain_order_is_reported() {
        let mut c = good_chain();
        c[2].owner = bob();
        c[1].reparse = true;
        assert_eq!(
            evaluate(&c, &set()),
            Err(Refusal::Reparse {
                path: "C:\\u".into()
            })
        );
    }

    #[test]
    fn inherited_non_inherit_only_ace_counts() {
        let mut c = good_chain();
        c[2].dacl.as_mut().unwrap().push(Ace {
            sid: bob(),
            mask: GENERIC_ALL,
            inherit_only: false,
            allow: true,
        });
        assert!(matches!(evaluate(&c, &set()), Err(Refusal::Access { .. })));
    }

    #[test]
    fn generic_read_execute_accepted_and_generic_write_on_ancestor() {
        let mut c = good_chain();
        c[2].dacl.as_mut().unwrap().push(allow(bob(), GXGR));
        c[1].dacl.as_mut().unwrap().push(allow(bob(), GXGR));
        c[1].dacl
            .as_mut()
            .unwrap()
            .push(allow(bob(), GENERIC_WRITE));
        assert_eq!(evaluate(&c, &set()), Ok(()));
    }

    #[test]
    fn components_shapes() {
        let root = vec!["C:\\".to_string()];
        assert_eq!(components("C:\\").unwrap(), root);
        assert_eq!(components("C:\\a\\").unwrap(), vec!["C:\\", "C:\\a"]);
        assert_eq!(
            components("C:/a/b").unwrap(),
            vec!["C:\\", "C:\\a", "C:\\a\\b"]
        );
        assert_eq!(components("C:\\\\a").unwrap(), vec!["C:\\", "C:\\a"]);
        assert_eq!(components("c:\\a").unwrap(), vec!["C:\\", "C:\\a"]);
        assert_eq!(
            components("C:\\Users\\Nguyễn Văn\\x").unwrap(),
            vec![
                "C:\\",
                "C:\\Users",
                "C:\\Users\\Nguyễn Văn",
                "C:\\Users\\Nguyễn Văn\\x"
            ]
        );
    }

    #[test]
    fn components_refusals() {
        for bad in [
            "C:",
            "",
            "\\\\.\\C:\\a",
            "\\\\?\\UNC\\server\\share",
            "C:\\a\\..\\b",
        ] {
            assert!(
                matches!(components(bad), Err(Refusal::Remote { .. })),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn components_refuse_unsafe_segments() {
        for bad in [
            "C:\\a:ads",
            "C:\\a\\f.txt:s:$DATA",
            "C:\\a*",
            "C:\\a?",
            "C:\\a<",
            "C:\\a>",
            "C:\\a|b",
            "C:\\a\"b",
            "C:\\a.",
            "C:\\a ",
            "C:\\a. \\b",
        ] {
            assert_eq!(components(bad), Err(mal(bad)), "{bad:?}");
        }
    }

    #[test]
    fn parse_sid_accepts_and_rejects() {
        assert_eq!(parse_sid("S-1-5-18"), Some(s("S-1-5-18")));
        assert!(parse_sid(TRUSTED_INSTALLER).is_some());
        for bad in [
            "",
            "S-1-5",
            "S-1-",
            "S-2-5-18",
            "S-1-5-x",
            "S-1-5-",
            "s-1-5-18",
            "S-1-5-18 ",
        ] {
            assert_eq!(parse_sid(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn measured_runner_profile_chain_passes() {
        let la = Sid("S-1-5-21-3289802278-1561746829-1498425070-500".into());
        let ti = s(TRUSTED_INSTALLER);
        let (au, bu, wd) = (s("S-1-5-11"), s("S-1-5-32-545"), s(EVERYONE));
        let ac = s("S-1-15-3-65536-4045685566");
        let io = |sid: Sid, mask| Ace {
            sid,
            mask,
            inherit_only: true,
            allow: true,
        };
        let fa3 = || {
            vec![
                allow(s(SYSTEM), FA),
                allow(s(ADMINISTRATORS), FA),
                allow(la.clone(), FA),
            ]
        };
        let ba = s(ADMINISTRATORS);
        let chain = vec![
            d(
                "C:\\",
                ti.clone(),
                vec![
                    io(
                        au.clone(),
                        0x1000_0000 | 0x2_0000 | 0x2000_0000 | 0x8000_0000 | 0x4000_0000,
                    ),
                    allow(au, LC),
                    allow(s(SYSTEM), FA),
                    allow(s(ADMINISTRATORS), FA),
                    allow(bu.clone(), RX),
                    allow(s("S-1-15-3-65536-1888954469"), 0x1000a1),
                ],
                false,
                true,
            ),
            d(
                "C:\\Users",
                s(SYSTEM),
                vec![
                    io(wd.clone(), GXGR),
                    allow(wd, RX),
                    allow(s(SYSTEM), FA),
                    allow(s(ADMINISTRATORS), FA),
                    allow(bu.clone(), RX),
                    io(bu, GXGR),
                    allow(ac.clone(), 0x100021),
                ],
                false,
                true,
            ),
            d(
                "C:\\Users\\runneradmin",
                s(SYSTEM),
                {
                    let mut v = fa3();
                    v.push(allow(s("S-1-15-3-65536-599108337"), 0x100020));
                    v
                },
                false,
                true,
            ),
            d(
                "C:\\Users\\runneradmin\\AppData",
                ba.clone(),
                fa3(),
                false,
                true,
            ),
            d(
                "C:\\Users\\runneradmin\\AppData\\Local",
                ba.clone(),
                fa3(),
                false,
                true,
            ),
            d(
                "C:\\Users\\runneradmin\\AppData\\Local\\iroh-tunnel",
                ba.clone(),
                fa3(),
                false,
                true,
            ),
            d(
                "C:\\Users\\runneradmin\\AppData\\Local\\iroh-tunnel\\config",
                ba,
                fa3(),
                false,
                false,
            ),
        ];
        assert_eq!(evaluate(&chain, &TrustedSet::for_user_files(&la)), Ok(()));
    }
}
