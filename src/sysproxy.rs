//! Bật/tắt cấu hình proxy tự động (PAC) của hệ điều hành.

use std::process::Command;

fn run(cmd: &str, args: &[&str]) -> bool {
    match Command::new(cmd).args(args).output() {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            eprintln!("[sysproxy] {cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim());
            false
        }
        Err(e) => {
            eprintln!("[sysproxy] không chạy được {cmd}: {e}");
            false
        }
    }
}

#[cfg(target_os = "macos")]
fn services() -> Vec<String> {
    Command::new("networksetup")
        .arg("-listallnetworkservices")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .skip(1) // dòng giới thiệu
                .filter(|l| !l.starts_with('*')) // dịch vụ bị tắt
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
pub fn set_pac(url: &str) -> bool {
    let mut ok = true;
    for s in services() {
        ok &= run("networksetup", &["-setautoproxyurl", &s, url]);
        ok &= run("networksetup", &["-setautoproxystate", &s, "on"]);
    }
    ok
}

#[cfg(target_os = "macos")]
pub fn clear() {
    for s in services() {
        run("networksetup", &["-setautoproxystate", &s, "off"]);
    }
}

#[cfg(windows)]
const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";

#[cfg(windows)]
pub fn set_pac(url: &str) -> bool {
    run("reg", &["add", KEY, "/v", "AutoConfigURL", "/t", "REG_SZ", "/d", url, "/f"])
}

#[cfg(windows)]
pub fn clear() {
    run("reg", &["delete", KEY, "/v", "AutoConfigURL", "/f"]);
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn set_pac(url: &str) -> bool {
    run("gsettings", &["set", "org.gnome.system.proxy", "autoconfig-url", url])
        && run("gsettings", &["set", "org.gnome.system.proxy", "mode", "auto"])
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn clear() {
    run("gsettings", &["set", "org.gnome.system.proxy", "mode", "none"]);
}
