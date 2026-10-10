//! Ghi/gỡ một khối tên miền Steam trong file hosts của hệ điều hành.

use anyhow::{Context, Result};
use std::net::IpAddr;
use std::path::PathBuf;

const BEGIN: &str = "# >>> steam-unblock";
const END: &str = "# <<< steam-unblock";

#[cfg(windows)]
const NL: &str = "\r\n";
#[cfg(not(windows))]
const NL: &str = "\n";

pub fn path() -> PathBuf {
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        PathBuf::from(root).join(r"System32\drivers\etc\hosts")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/etc/hosts")
    }
}

/// Bỏ khối của steam-unblock (nếu có) khỏi nội dung file hosts.
fn strip_block(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut inside = false;
    for line in content.lines() {
        let t = line.trim();
        if t == BEGIN {
            inside = true;
        } else if t == END {
            inside = false;
        } else if !inside {
            out.push_str(line);
            out.push_str(NL);
        }
    }
    out
}

fn write(content: &str) -> Result<()> {
    let p = path();
    std::fs::write(&p, content).with_context(|| {
        format!(
            "không ghi được {} (cần chạy terminal bằng Administrator / sudo; antivirus cũng có thể chặn sửa file hosts)",
            p.display()
        )
    })?;
    flush_dns();
    Ok(())
}

/// Thêm (hoặc thay) khối tên miền trỏ về `ip`.
pub fn install(ip: IpAddr, hosts: &[String]) -> Result<()> {
    let p = path();
    let old = std::fs::read_to_string(&p).with_context(|| format!("không đọc được {}", p.display()))?;
    let mut new = strip_block(&old);
    if !new.is_empty() && !new.ends_with('\n') {
        new.push_str(NL);
    }
    new.push_str(BEGIN);
    new.push_str(NL);
    for h in hosts {
        new.push_str(&format!("{ip} {h}{NL}"));
    }
    new.push_str(END);
    new.push_str(NL);
    write(&new)
}

/// Gỡ khối tên miền; không làm gì nếu không có.
pub fn uninstall() -> Result<()> {
    let p = path();
    let old = std::fs::read_to_string(&p).with_context(|| format!("không đọc được {}", p.display()))?;
    if !old.contains(BEGIN) {
        return Ok(());
    }
    write(&strip_block(&old))
}

fn flush_dns() {
    #[cfg(windows)]
    let _ = std::process::Command::new("ipconfig").arg("/flushdns").output();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("dscacheutil").arg("-flushcache").output();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_keeps_other_lines() {
        let s = format!("127.0.0.1 localhost{NL}{BEGIN}{NL}127.0.0.1 a.com{NL}{END}{NL}1.2.3.4 b{NL}");
        assert_eq!(strip_block(&s), format!("127.0.0.1 localhost{NL}1.2.3.4 b{NL}"));
    }
}
