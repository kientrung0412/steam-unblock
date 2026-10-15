# steam-unblock

Công cụ dòng lệnh viết bằng Rust giúp truy cập Steam khi bị nhà mạng chặn. Chạy trên macOS, Linux và Windows.

## Cách hoạt động

| Kiểu chặn | Cách vượt |
|---|---|
| DNS bị "đầu độc" | Phân giải tên miền qua DNS-over-HTTPS (1.1.1.1, dự phòng 8.8.8.8) |
| Lọc SNI trên HTTPS (DPI) | Chia TLS ClientHello thành 2 bản ghi, gửi thành 2 gói TCP, điểm cắt nằm giữa tên miền |
| Lọc header `Host` trên HTTP | Đổi thành `hOSt:` và cắt gói ở giữa tên miền |

Không cần máy chủ trung gian hay VPN: dữ liệu vẫn đi thẳng tới Steam và vẫn được mã hoá TLS đầu-cuối.

## Build

```bash
cargo build --release
# file chạy: target/release/steam-unblock  (Windows: steam-unblock.exe)
```

Push lên GitHub thì workflow `.github/workflows/release.yml` sẽ build sẵn cho cả 3 hệ điều hành.

## Sử dụng

### Giao diện (dễ nhất)
Nhấp đúp `steam-unblock` (hoặc chạy không tham số / `steam-unblock gui`) để mở cửa sổ:

- **Trình duyệt / web Steam**: bấm *Bắt đầu*, tự đặt PAC cho hệ thống; bấm *Dừng* sẽ gỡ.
- **Steam client**: cần quyền Administrator (trên Windows có nút *Chạy lại bằng Administrator*).
  Thoát Steam hoàn toàn, bấm *Bắt đầu*, rồi mở lại Steam. Bấm *Dừng* hoặc đóng cửa sổ sẽ trả file hosts như cũ.
- *Kiểm tra chặn* và *Gỡ hosts* tương ứng với lệnh `check` và `unhosts`.

Không có màn hình (máy chủ, SSH) thì chạy không tham số sẽ tự chạy proxy như trước.

### Dòng lệnh

```bash
steam-unblock check          # kiểm tra Steam đang bị chặn kiểu gì
steam-unblock proxy          # chạy proxy tại 127.0.0.1:8899
steam-unblock proxy --system # chạy proxy và tự cấu hình PAC cho hệ thống (Ctrl+C sẽ gỡ cấu hình)
steam-unblock hosts          # cho Steam client: file hosts + proxy trong suốt (cần Administrator/sudo)
steam-unblock unhosts        # gỡ khối tên miền Steam khỏi file hosts nếu lần trước thoát không sạch
```

### Trình duyệt / web Steam
Cách 1: dùng `steam-unblock proxy --system`.
Cách 2: tự đặt "Automatic proxy configuration URL" là `http://127.0.0.1:8899/proxy.pac`.
Khi dùng PAC thì chỉ các tên miền Steam đi qua proxy, các trang khác vẫn đi thẳng.

### Steam client (ứng dụng)
Steam client (bộ cập nhật `steam.exe` và kết nối CM server) **không dùng PAC**, nên với `proxy --system`
trình duyệt vào được Steam nhưng client vẫn báo *"Steam needs to be online to update"*. Hãy dùng chế độ `hosts`:

1. Tắt `proxy --system` nếu đang chạy, thoát Steam hoàn toàn (kể cả ở khay hệ thống).
2. Mở terminal bằng **Administrator** (macOS/Linux: `sudo`), chạy `steam-unblock hosts` và để cửa sổ mở.
3. Mở Steam.

Chế độ này ghi các tên miền Steam (cả danh sách CM/content server lấy từ Steam API) vào file hosts, trỏ về
`127.0.0.1`, rồi lắng nghe cổng 80/443 (và cổng CM). Mỗi kết nối được đọc tên miền từ SNI/Host, phân giải
bằng DoH và chuyển tiếp tới Steam với ClientHello đã chia nhỏ. TLS vẫn đầu-cuối, không cần cài chứng chỉ.
Nhấn Ctrl+C để dừng: file hosts được trả lại như cũ (nếu bị tắt ngang, chạy `steam-unblock unhosts`).

Nếu báo không mở được cổng 80/443: có chương trình khác chiếm cổng (IIS, XAMPP…), thử `--ip 127.0.0.2`.

Cách khác:
- Nếu nhà mạng chỉ chặn DNS: chạy `sudo steam-unblock dns` (Windows thì mở terminal bằng Administrator),
  rồi đặt DNS của máy thành `127.0.0.1`. Cách này áp dụng cho mọi ứng dụng.
  Lưu ý: chế độ `dns` **không** vượt được lọc SNI. Gặp trường hợp đó thì dùng `proxy`.

### Tuỳ chọn khác
```
proxy -p 9000           đổi cổng
proxy --host 0.0.0.0    chia sẻ proxy cho các máy khác trong mạng LAN
proxy --no-fragment     chỉ dùng DoH, không chia nhỏ gói
proxy -v                in thời gian dns/connect của từng kết nối (tìm chỗ chậm)
pac -p 8899             in nội dung file PAC
hosts --no-dynamic      không tải danh sách CM/content server, chỉ dùng danh sách cố định
hosts -v                in thời gian kết nối của từng tên miền
```
