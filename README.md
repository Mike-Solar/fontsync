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

## 说明

- Linux 托盘图标使用系统主题图标 `preferences-desktop-font`。
- Windows 托盘图标使用系统默认应用图标（`IDI_APPLICATION`）。
- 如需自定义，可在 `src/gui.rs` 中修改。
