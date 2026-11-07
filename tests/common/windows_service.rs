//! Windows Service helpers shared by the service end-to-end tests: SCM and
//! registry readers keyed by service name.

use anyhow::{ensure, Result};
use std::process::{Command, Output};

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// `sc.exe <verb> <name>`.
pub fn sc(verb: &str, name: &str) -> Output {
    Command::new("sc.exe")
        .args([verb, name])
        .output()
        .expect("run sc.exe")
}

pub fn service_exists(name: &str) -> bool {
    sc("query", name).status.success()
}

/// Whether the service is absent or in the STOPPED state.
pub fn service_stopped_or_absent(name: &str) -> bool {
    let out = sc("query", name);
    !out.status.success() || stdout(&out).contains("STOPPED")
}

/// How many services carry exactly `name` (case-insensitive).
pub fn service_count(name: &str) -> Result<usize> {
    let out = Command::new("sc.exe")
        .args(["query", "type=", "service", "state=", "all"])
        .output()?;
    ensure!(out.status.success(), "sc query: {}", stdout(&out));
    Ok(stdout(&out)
        .lines()
        .filter_map(|l| l.trim().strip_prefix("SERVICE_NAME:"))
        .filter(|n| n.trim().eq_ignore_ascii_case(name))
        .count())
}

pub fn registry_key(name: &str) -> String {
    format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{name}")
}

/// The data of a `reg query` line of the given type.
pub fn reg_value(output: &str, kind: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (_, data) = line.split_once(kind)?;
        Some(data.trim().to_owned())
    })
}

/// Raw `reg query` output for one value of the service key.
fn reg_query(name: &str, value: &str) -> Result<String> {
    let out = Command::new("reg")
        .args(["query", &registry_key(name), "/v", value])
        .output()?;
    ensure!(out.status.success(), "reg query {value}: {}", stderr(&out));
    Ok(stdout(&out))
}

/// A string value (`REG_SZ` or `REG_EXPAND_SZ`) of the service key.
pub fn reg_string(name: &str, value: &str) -> Result<String> {
    let text = reg_query(name, value)?;
    reg_value(&text, "REG_EXPAND_SZ")
        .or_else(|| reg_value(&text, "REG_SZ"))
        .ok_or_else(|| anyhow::anyhow!("no {value} in {text}"))
}

/// A `REG_DWORD` value of the service key.
pub fn reg_dword(name: &str, value: &str) -> Result<u32> {
    let text = reg_query(name, value)?;
    let data =
        reg_value(&text, "REG_DWORD").ok_or_else(|| anyhow::anyhow!("no {value} in {text}"))?;
    Ok(u32::from_str_radix(data.trim_start_matches("0x"), 16)?)
}

/// `ImagePath` of the service, as the SCM stores it.
pub fn image_path(name: &str) -> Result<String> {
    reg_string(name, "ImagePath")
}

/// The `Environment` entries (`KEY=value`) of the service.
pub fn service_env(name: &str) -> Result<Vec<String>> {
    let text = reg_query(name, "Environment")?;
    Ok(reg_value(&text, "REG_MULTI_SZ")
        .unwrap_or_default()
        .split("\\0")
        .filter(|e| !e.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Upsert `extra` into the service's `Environment` (REG_MULTI_SZ). The change
/// applies from the next start. Values must not contain the literal `\0`
/// sequence: `reg` reads it as the entry separator.
pub fn set_service_env(name: &str, extra: &[(&str, &str)]) -> Result<()> {
    let mut entries = service_env(name)?;
    for (key, value) in extra {
        entries.retain(|e| !e.starts_with(&format!("{key}=")));
        entries.push(format!("{key}={value}"));
    }
    write_service_env(name, &entries)
}

/// Remove `key` from the service's `Environment`; applies from the next start.
pub fn remove_service_env(name: &str, key: &str) -> Result<()> {
    let mut entries = service_env(name)?;
    let before = entries.len();
    entries.retain(|e| !e.starts_with(&format!("{key}=")));
    ensure!(entries.len() < before, "{key} is not in the environment");
    write_service_env(name, &entries)
}

fn write_service_env(name: &str, entries: &[String]) -> Result<()> {
    let out = Command::new("reg")
        .args([
            "add",
            &registry_key(name),
            "/v",
            "Environment",
            "/t",
            "REG_MULTI_SZ",
            "/d",
            &entries.join("\\0"),
            "/f",
        ])
        .output()?;
    ensure!(
        out.status.success(),
        "reg add Environment: {}",
        stderr(&out)
    );
    Ok(())
}

/// Overwrite the service's `Description` (where the pin record lives).
pub fn set_description(name: &str, text: &str) -> Result<()> {
    let out = Command::new("reg")
        .args([
            "add",
            &registry_key(name),
            "/v",
            "Description",
            "/t",
            "REG_SZ",
            "/d",
            text,
            "/f",
        ])
        .output()?;
    ensure!(
        out.status.success(),
        "reg add Description: {}",
        stderr(&out)
    );
    Ok(())
}

/// The SID of the user running the test (`whoami /user`, SID column only).
pub fn runner_sid() -> Result<String> {
    let out = Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()?;
    ensure!(out.status.success(), "whoami: {}", stderr(&out));
    let text = stdout(&out);
    let sid = text
        .trim()
        .rsplit(',')
        .next()
        .map(|s| s.trim().trim_matches('"').to_owned())
        .filter(|s| s.starts_with("S-1-"));
    sid.ok_or_else(|| anyhow::anyhow!("no SID in whoami output: {text}"))
}

/// The config path of a pin record (`iroh-tunnel-pin config=<%XX> sid=..`),
/// unescaped and normalised (lowercase, `/` separators) for comparison.
pub fn pin_config(description: &str) -> Option<String> {
    let token = description
        .split_whitespace()
        .find_map(|t| t.strip_prefix("config="))?;
    let bytes = token.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(token.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(
        String::from_utf8_lossy(&out)
            .replace('\\', "/")
            .to_lowercase(),
    )
}
