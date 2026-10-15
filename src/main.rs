mod check;
mod dns;
mod doh;
mod gui;
mod hosts;
mod proxy;
mod sysproxy;
mod transparent;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

/// Truy cập Steam khi bị nhà mạng chặn (DoH + chia nhỏ TLS ClientHello).
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Chạy proxy HTTP/HTTPS cục bộ
    Proxy {
        /// Cổng lắng nghe
        #[arg(short, long, default_value_t = 8899)]
        port: u16,
        /// Địa chỉ lắng nghe (0.0.0.0 để chia sẻ cho máy khác trong mạng LAN)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Tắt chia nhỏ ClientHello
        #[arg(long)]
        no_fragment: bool,
        /// Tự đặt PAC vào cấu hình proxy của hệ điều hành (tắt khi thoát bằng Ctrl+C)
        #[arg(short, long)]
        system: bool,
        /// In thời gian từng bước của mỗi kết nối
        #[arg(short, long)]
        verbose: bool,
        /// Số mảnh cắt tên miền SNI trong ClientHello
        #[arg(long, default_value_t = 4)]
        pieces: usize,
    },
    /// Dành cho Steam client: trỏ tên miền Steam về máy qua file hosts + proxy trong suốt cổng 80/443
    /// (cần Administrator/sudo). Ctrl+C sẽ gỡ file hosts.
    Hosts {
        /// IP loopback để lắng nghe và ghi vào file hosts
        #[arg(long, default_value = "127.0.0.1")]
        ip: IpAddr,
        /// Tắt chia nhỏ ClientHello
        #[arg(long)]
        no_fragment: bool,
        /// Không tải danh sách CM/content server từ Steam API
        #[arg(long)]
        no_dynamic: bool,
        /// In thời gian từng kết nối
        #[arg(short, long)]
        verbose: bool,
        /// Số mảnh cắt tên miền SNI trong ClientHello
        #[arg(long, default_value_t = 4)]
        pieces: usize,
    },
    /// Gỡ khối tên miền Steam khỏi file hosts (khi lần trước thoát không sạch)
    Unhosts,
    /// Chạy DNS server cục bộ chuyển tiếp qua DNS-over-HTTPS (cho Steam client)
    Dns {
        #[arg(short, long, default_value = "127.0.0.1:53")]
        bind: SocketAddr,
    },
    /// Kiểm tra xem Steam đang bị chặn kiểu gì
    Check,
    /// Mở giao diện đồ hoạ (mặc định khi chạy không có tham số)
    Gui,
    /// In nội dung file PAC
    Pac {
        #[arg(short, long, default_value_t = 8899)]
        port: u16,
    },
}

/// Chờ tín hiệu dừng: Ctrl+C, hoặc stdin bị đóng khi chạy dưới GUI.
pub async fn shutdown() {
    if std::env::var_os(gui::ENV).is_none() {
        let _ = tokio::signal::ctrl_c().await;
        return;
    }
    let stdin_closed = async {
        use tokio::io::AsyncReadExt;
        let mut stdin = tokio::io::stdin();
        let mut buf = [0u8; 64];
        while matches!(stdin.read(&mut buf).await, Ok(n) if n > 0) {}
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = stdin_closed => {}
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if matches!(cli.cmd, None | Some(Cmd::Gui)) {
        let explicit = cli.cmd.is_some();
        if !explicit {
            gui::hide_own_console();
        }
        match gui::run() {
            Ok(()) => return Ok(()),
            Err(e) if explicit => anyhow::bail!("không mở được giao diện: {e}"),
            // Không có màn hình (máy chủ, SSH…): chạy proxy như trước.
            Err(e) => eprintln!("Không mở được giao diện ({e}), chạy proxy."),
        }
    }
    tokio::runtime::Runtime::new()?.block_on(run(cli))
}

async fn run(cli: Cli) -> Result<()> {
    let doh = Arc::new(doh::Doh::new()?);

    match cli.cmd.unwrap_or(Cmd::Proxy {
        port: 8899,
        host: "127.0.0.1".into(),
        no_fragment: false,
        system: false,
        verbose: false,
        pieces: 4,
    }) {
        Cmd::Proxy { port, host, no_fragment, system, verbose, pieces } => {
            let bind: SocketAddr = format!("{host}:{port}").parse()?;
            doh.prefetch();
            let cfg = proxy::Config { port, fragment: !no_fragment,
                verbose,
                pieces,
            };
            if system {
                let url = format!("http://127.0.0.1:{port}/proxy.pac");
                if sysproxy::set_pac(&url) {
                    println!("Đã đặt proxy tự động của hệ thống: {url}");
                } else {
                    eprintln!("Không đặt được proxy hệ thống, hãy cấu hình thủ công.");
                }
            }
            tokio::select! {
                r = proxy::run(bind, cfg, doh) => r?,
                _ = shutdown() => println!("\nĐang thoát..."),
            }
            if system {
                sysproxy::clear();
                println!("Đã tắt proxy tự động của hệ thống.");
            }
        }
        Cmd::Hosts { ip, no_fragment, no_dynamic, verbose, pieces } => {
            let cfg = proxy::Config { port: 0, fragment: !no_fragment, verbose, pieces };
            transparent::run(transparent::Options { ip, cfg, dynamic: !no_dynamic }, doh).await?;
        }
        Cmd::Unhosts => {
            hosts::uninstall()?;
            println!("Đã gỡ tên miền Steam khỏi {}", hosts::path().display());
        }
        Cmd::Dns { bind } => {
            tokio::select! {
                r = dns::run(bind, doh) => r?,
                _ = shutdown() => println!("\nĐang thoát..."),
            }
        }
        Cmd::Check => check::run(&doh).await,
        Cmd::Gui => unreachable!(),
        Cmd::Pac { port } => print!("{}", proxy::pac(port)),
    }
    Ok(())
}
