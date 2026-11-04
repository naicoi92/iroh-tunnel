//! Runtime engine: endpoint binding, role loops, tunnels and their typed
//! settings/observations. Everything here is host-agnostic: nothing under
//! `src/engine/` may name a crate path outside the engine subtree, so the
//! subtree can become its own crate without back-references.

pub mod access;
pub mod conn_path;
pub mod endpoint;
pub mod observation;
pub mod pipe;
pub mod proto;
pub mod ready;
pub mod runtime;
pub mod serve;
pub mod settings;
pub mod shutdown;
pub mod testing;
pub mod udp;

pub use access::run_access;
pub use endpoint::{bind_access, bind_serve};
pub use runtime::RunCompletion;
pub use serve::run_serve;
pub use shutdown::RunContext;

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn engine_module_has_no_outward_crate_paths() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/engine");
        let mut files = Vec::new();
        rust_files(&root, &mut files);
        let mut offenders = Vec::new();
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for (n, line) in text.lines().enumerate() {
                let needle = concat!("cr", "ate::");
                let allowed = concat!("cr", "ate::engine::");
                let mut rest = line;
                while let Some(at) = rest.find(needle) {
                    let tail = &rest[at..];
                    rest = &rest[at + needle.len()..];
                    if !tail.starts_with(allowed) {
                        offenders.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "outward crate paths:\n{}",
            offenders.join("\n")
        );
    }
}
