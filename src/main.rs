mod check;
mod dns;
mod doh;
mod proxy;
mod sysproxy;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
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
    /// Chạy proxy HTTP/HTTPS cục bộ (mặc định)
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
    /// Chạy DNS server cục bộ chuyển tiếp qua DNS-over-HTTPS (cho Steam client)
    Dns {
        #[arg(short, long, default_value = "127.0.0.1:53")]
        bind: SocketAddr,
    },
    /// Kiểm tra xem Steam đang bị chặn kiểu gì
    Check,
    /// In nội dung file PAC
    Pac {
        #[arg(short, long, default_value_t = 8899)]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
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
                _ = tokio::signal::ctrl_c() => println!("\nĐang thoát..."),
            }
            if system {
                sysproxy::clear();
                println!("Đã tắt proxy tự động của hệ thống.");
            }
        }
        Cmd::Dns { bind } => {
            tokio::select! {
                r = dns::run(bind, doh) => r?,
                _ = tokio::signal::ctrl_c() => println!("\nĐang thoát..."),
            }
        }
        Cmd::Check => check::run(&doh).await,
        Cmd::Pac { port } => print!("{}", proxy::pac(port)),
    }
    Ok(())
}
