# FontSync

FontSync 是一个跨平台字体同步工具，支持 HTTP/WebSocket 同步与 GUI 模式。程序默认启动 GUI，并可最小化到系统托盘运行。

## 功能

- 扫描与同步本地字体目录
- HTTP 服务端/客户端同步
- WebSocket 实时通知
- GUI 界面，支持最小化到托盘

## 构建与运行

### 本地构建

```bash
cargo build --release
```

### 直接运行

```bash
cargo run
```

默认无参数时会打开 GUI（可最小化到托盘）。

### 无 GUI 模式

```bash
cargo run -- --no-gui serve --host 127.0.0.1 --port 8080 --font-dir ./fonts
```

### 常用命令

```bash
# 启动服务端
cargo run -- serve --host 127.0.0.1 --port 8080 --font-dir ./fonts

# 监控模式
cargo run -- monitor --server-url ws://localhost:8080

# 一次性同步
cargo run -- sync --server-url http://localhost:8080 --local-dir ./local_fonts
```

## 测试

```bash
cargo test
```

## GitHub Actions

- 自动化测试：Windows / Linux
- 自动打包发布：推送 `v*` 标签时生成 release，并上传二进制
  - Linux 发布为 `x86_64-unknown-linux-musl` 静态二进制，并附带 `fontsync.service` 模板

## 说明

- Linux 托盘图标使用系统主题图标 `preferences-desktop-font`。
- Windows 托盘图标使用系统默认应用图标（`IDI_APPLICATION`）。
- 如需自定义，可在 `src/gui.rs` 中修改。

## systemd 服务端运行（Linux）

发布的 Linux 压缩包内包含 `fontsync.service` 模板，可用于启动服务端。

```bash
# 解压
tar -xzf fontsync-linux-x86_64-musl.tar.gz

# 安装二进制
sudo install -m 0755 fontsync /usr/local/bin/fontsync

# 创建运行用户与数据目录
sudo useradd --system --no-create-home --shell /usr/sbin/nologin fontsync
sudo mkdir -p /var/lib/fontsync/fonts
sudo chown -R fontsync:fontsync /var/lib/fontsync

# 安装并启用服务
sudo install -m 0644 fontsync.service /etc/systemd/system/fontsync.service
sudo systemctl daemon-reload
sudo systemctl enable --now fontsync.service

# 查看状态与日志
systemctl status fontsync.service
journalctl -u fontsync.service -f
```

如需更改端口/目录，请编辑 `fontsync.service` 中的 `ExecStart` 参数。
