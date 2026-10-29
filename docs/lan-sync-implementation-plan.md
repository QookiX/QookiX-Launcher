# 局域网直传同步 — 实施计划

> 基于 2026-10-27 对桌面端（`QookiX-Launcher`）与安卓端（`QookiX-Launcher-Android`）代码审计结果制定。

---

## 一、架构现状（审计结论）

### 桌面端

| 能力 | 状态 | 说明 |
|---|---|---|
| Deep Link `qookix://` | ✅ 已注册 | `tauri-plugin-deep-link`，路由在 `deepLink.ts`，支持 `launch/<alias>` / `skin` / `settings` 等 |
| HTTP 服务端 | ❌ 无 | 有 `reqwest`（客户端），无任何 inbound server。`tokio` 已有网络 feature |
| 实例导出 `.qkxinst` | ✅ 已有 | `instance_share.rs`（1167 行）：预览 → 勾选 → zip 打包 → 导入还原 |
| 云存档 | ✅ 已有 | `cloud_sync/`：GitHub Releases 备份，pack → zip → sha256 → 上传，有进度事件 |
| QR 码 | ❌ 无 | 需新增（推荐前端 JS 库，不动 Rust） |
| CSP | ⚠️ 限制 | `connect-src` 不含 `http://192.168.x.x`，需拓宽或走 Tauri command |
| 长进程管理 | ✅ 有模式 | `server_pids` / `server_senders` 管理 MC 服进程，可参考 |

### 安卓端

| 能力 | 状态 | 说明 |
|---|---|---|
| Deep Link `qookix://` | ❌ 未注册 | `MinecraftProfile.alias` 字段已对齐，但 `AndroidManifest.xml` 无 `qookix` scheme intent-filter |
| HTTP 服务端 | ❌ 无框架 | 有 `TcpListener`（端口检测 / 休眠唤醒），有 `tokio` full features，可加 `axum` |
| HTTP 客户端 | ✅ `reqwest` | 已有，可直接用于下载 |
| 命令分发 | ✅ JNI | `native_invoke(cmd, payload)` → 按 domain dispatch，新增命令只需加 dispatch 分支 |
| 网络权限 | ✅ 已有 | `INTERNET` + `CHANGE_WIFI_MULTICAST_STATE`（Terracotta 用） |
| 存储权限 | ✅ 已有 | `MANAGE_EXTERNAL_STORAGE`，可直接读写文件 |
| QR 扫码 | ❌ 无 | 需加相机 + ZXing / ML Kit |
| 实例格式 | ✅ 对齐 | `MinecraftProfile` ↔ `Instance`，serde alias 兼容 |

---

## 二、整体设计

```
┌──── 桌面端 (PC) ────────────┐         ┌──── 安卓端 (手机) ────────────┐
│                              │         │                              │
│  ① 用户开「局域网同步」面板   │         │                              │
│  ② Rust 启 HTTP 服务         │         │                              │
│     bind 192.168.1.x:8765    │         │                              │
│  ③ 生成一次性 token          │         │                              │
│  ④ 前端渲染 QR 码            │         │                              │
│     qookix://lan/pair        │         │                              │
│       ?host=192.168.1.x      │         │                              │
│       &port=8765             │         │                              │
│       &token=a3f8b2c1        │         │                              │
│              │               │         │               ┌──────────────┤
│              │   用户扫码     │         │  ⑤ 扫码 │ 相机+ZXing      │
│              │ ─────────────►│         │  ⑥ intent 唤起 QookiX       │
│              │               │         │     deep link → 跳转同步页   │
│              │               │         │  ⑦ reqwest 连 PC HTTP 服务   │
│  ⑧ 验证 token ◄─────────────│─────────┤     GET /api/info           │
│     返回实例列表 │            │         │  ⑨ 显示 PC 实例列表          │
│              │ ─────────────►│         │  ⑩ 用户勾选 → 下载           │
│  ⑪ 流式发送 zip │            │         │     GET /api/instance/:id   │
│              │ ─────────────►│         │     → 解压到实例目录         │
│              │               │         │  ⑫ 完成 ✓                    │
└──────────────────────────────┘         └──────────────────────────────┘
```

### 传输内容

用户在手机端勾选要同步的实例，每个实例可选项：

| 内容 | 默认 | 大小 | 说明 |
|---|---|---|---|
| `instance.json` | ✅ 必传 | ~2 KB | 版本 + 加载器 + 模组列表 |
| `mods/` | ✅ | 几十~几百 MB | 直接传文件，免手机端重新下载 |
| `resourcepacks/` | 可选 | 几 MB | 资源包 |
| `shaderpacks/` | 可选 | 几 MB | 光影 |
| `saves/` | 可选 | 几十 MB~几 GB | 世界存档 |
| `config/` | 可选 | 几 MB | 模组配置 |
| `servers.dat` | 可选 | ~1 KB | 服务器列表 |
| `options.txt` | 可选 | ~5 KB | 游戏设置 |

### 安全

- HTTP 服务绑定局域网 IP（非 `0.0.0.0`），仅同网段可访问
- 一次性 Bearer token（32 字节随机 hex），面板关闭即停服务 + 失效
- 不持久化任何配对信息
- 传输不加密（局域网内无必要）

---

## 三、分步实施

### Phase 0：Deep Link 统一（前置依赖）

> 局域网同步的二维码里是 `qookix://lan/pair?...`，手机扫码后需要 deep link 唤起 QookiX 安卓端。没有这步，退化为手输 IP（体验差）。

#### 安卓端改动

**文件 1**：`android/app/src/main/AndroidManifest.xml`

在 `ComposeMainActivity` 的 `<intent-filter>` 后追加：

```xml
<intent-filter>
    <action android:name="android.intent.action.VIEW" />
    <category android:name="android.intent.category.DEFAULT" />
    <category android:name="android.intent.category.BROWSABLE" />
    <data android:scheme="qookix" />
</intent-filter>
```

**文件 2**：`ComposeMainActivity.kt`

`onNewIntent` / `onCreate` 中解析 `intent.data`，提取 `qookix://<host>/<path>?<query>`，通过 `PageHost` 路由：

```kotlin
fun handleDeepLink(uri: android.net.Uri) {
    when (uri.host) {
        "lan" -> {
            // qookix://lan/pair?host=192.168.1.x&port=8765&token=xxx
            val host = uri.getQueryParameter("host") ?: return
            val port = uri.getQueryParameter("port")?.toIntOrNull() ?: return
            val token = uri.getQueryParameter("token") ?: return
            pageHost.navigate("lan-sync", mapOf("host" to host, "port" to port, "token" to token))
        }
        "launch" -> { /* 启动实例 */ }
        "skin" -> { /* 跳转皮肤 */ }
        // ... 其他路由
    }
}
```

**工作量**：~1 天
**风险**：低，纯增量

---

### Phase 1：桌面端 HTTP 服务 + QR 配对

#### 1.1 新增依赖

**`src-tauri/Cargo.toml`**：

```toml
axum = "0.7"
tower-http = { version = "0.5", features = ["fs", "cors"] }
```

> `axum` 基于 `tokio`（已有），轻量 HTTP 框架。`tower-http` 提供 CORS 和静态文件服务中间件。

**`package.json`**：

```json
"qrcode": "^1.5.4"
```

> 前端 JS 库渲染 QR 码，不动 Rust。

#### 1.2 新增 Rust 模块

**`src-tauri/src/lan_sync.rs`**（新文件）：

```rust
//! 局域网直传同步：HTTP 文件服务 + 令牌配对。
//!
//! 流程：start_server → 绑定局域网 IP → 生成 token → 前端渲染 QR →
//!       手机扫码连接 → 验证 token → 浏览实例 → 下载 zip → 完成。

use axum::{extract::Path, http::HeaderMap, response::Json, routing::get, Router};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 同步服务状态
pub struct LanSyncServer {
    pub port: u16,
    pub token: String,
    pub local_ip: String,
    pub shutdown: tokio::sync::oneshot::Sender<()>,
}

/// 启动 HTTP 服务，返回服务信息（含 token 和局域网 IP）
pub async fn start_server(root: &Path) -> Result<LanSyncServer, String> {
    // 1. 获取局域网 IP（遍历网卡，取第一个非 loopback 的 IPv4）
    let local_ip = get_lan_ip()?;

    // 2. 生成一次性 token
    let token = uuid::Uuid::new_v4().simple().to_string();

    // 3. 绑定端口（0 = 系统分配）
    let listener = tokio::net::TcpListener::bind((local_ip.as_str(), 0)).await?;
    let port = listener.local_addr()?.port();

    // 4. 构建路由
    let state = Arc::new(SyncState { root: root.to_owned(), token: token.clone() });
    let app = Router::new()
        .route("/api/info", get(api_info))
        .route("/api/instances", get(list_instances))
        .route("/api/instances/:id", get(download_instance))
        .route("/api/instances/:id/tree", get(instance_tree))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state);

    // 5. 启动服务（带 shutdown 信号）
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async { let _ = shutdown_rx.await; })
            .await
    });

    Ok(LanSyncServer { port, token, local_ip, shutdown: shutdown_tx })
}
```

**API 端点**：

| 端点 | 方法 | 说明 |
|---|---|---|
| `/api/info` | GET | 返回设备名 + 版本号 + 实例数量 |
| `/api/instances` | GET | 返回实例列表（id / name / mc_version / loader / size） |
| `/api/instances/:id/tree` | GET | 返回实例文件树（供前端展示可同步内容） |
| `/api/instances/:id` | GET | 流式下载实例 zip（复用 `instance_share` 打包逻辑） |
| `/api/servers` | GET | 返回服务器列表 JSON |

**令牌验证中间件**：

```rust
async fn auth_middleware(
    State(state): State<Arc<SyncState>>,
    headers: HeaderMap,
    next: Next,
) -> Response {
    let auth = headers.get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    if auth != Some(&state.token) {
        return (axum::http::StatusCode::UNAUTHORIZED, "Invalid token").into_response();
    }
    next.run(next_request).await
}
```

**局域网 IP 检测**：

```rust
fn get_lan_ip() -> Result<String, String> {
    // 使用 std::net::UdpSocket 连接 8.8.8.8:80 不发包，
    // 取 local_addr 获得本机选路 IP。
    // 或遍历 sysinfo / std 网卡接口。
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    Ok(socket.local_addr()?.ip().to_string())
}
```

#### 1.3 新增 Tauri 命令

**`src-tauri/src/commands/lan_sync.rs`**（新文件）：

```rust
#[tauri::command]
pub async fn lan_sync_start(state: State<'_, AppState>) -> Result<Json, String> {
    // 启动 HTTP 服务，存入 AppState，返回 { ip, port, token, qrUrl }
}

#[tauri::command]
pub async fn lan_sync_stop(state: State<'_, AppState>) -> Result<(), String> {
    // 关闭服务，清理状态
}

#[tauri::command]
pub async fn lan_sync_status(state: State<'_, AppState>) -> Result<Json, String> {
    // 返回当前服务状态
}
```

**`src-tauri/src/lib.rs`**：注册 `mod lan_sync` + 3 个命令

#### 1.4 前端 UI

**`src/components/LanSyncPanel.vue`**（新文件）：

```
┌─────────────────────────────────────┐
│        局域网同步                    │
│                                     │
│   ┌───────────┐   状态：等待连接    │
│   │  QR Code  │                     │
│   │  ▓▓▓▓▓▓▓ │   地址：192.168.1.100│
│   │  ▓▓▓▓▓▓▓ │   端口：8765        │
│   │  ▓▓▓▓▓▓▓ │                     │
│   └───────────┘   [停止服务]        │
│                                     │
│   手机扫码后自动连接                │
│   已连接设备：1 台                  │
└─────────────────────────────────────┘
```

- 用 `qrcode` JS 库渲染二维码
- QR 内容：`qookix://lan/pair?host=${ip}&port=${port}&token=${token}`
- 轮询 `lan_sync_status` 显示连接状态

**`src/composables/deepLink.ts`**：添加 `qookix://lan/...` 路由（桌面端被自己 QR 扫到时的兜底）

**`src/views/SettingsView.vue`**：设置页加「局域网同步」入口

#### 1.5 CSP 拓宽

**`src-tauri/tauri.conf.json`**：

```json
"csp": "... connect-src ipc: http://ipc.localhost asset: http://asset.localhost https://asset.localhost http://192.168.*.* http://localhost:* ..."
```

> 或更安全：不走前端 fetch，全部通过 Tauri command 中转（CSP 不用改）。推荐后者。

**工作量**：~3 天
**风险**：低-中。Windows 防火墙首次弹窗需用户允许。

---

### Phase 2：安卓端 HTTP 客户端 + 扫码接收

#### 2.1 QR 扫码

**`android/app/build.gradle.kts`**：

```kotlin
dependencies {
    // ZXing 纯扫码（轻量，不拉全 ML Kit）
    implementation("com.google.zxing:core:3.5.3")
    implementation("io.github.g0dkar:qrcode-kotlin:4.1.1") // 或用 CameraX + ZXing
}
```

> 或用 `mlkit-barcode-scanning`（Google 官方，体积大但体验好）。推荐 ZXing 轻量版。

**新增 `LanSyncScanner.kt`**：CameraX 预览 + ZXing 解码 → 提取 `qookix://lan/pair?...` → 调 `handleDeepLink`

#### 2.2 Rust HTTP 客户端

**`src-tauri/src/lan_sync.rs`**（安卓端新文件）：

```rust
//! 局域网同步客户端：连接 PC HTTP 服务 → 浏览实例 → 下载。

/// 连接 PC，验证 token，返回设备信息 + 实例列表
pub async fn connect(host: &str, port: u16, token: &str) -> Result<DeviceInfo, String> {
    let url = format!("http://{host}:{port}/api/info");
    let resp = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send().await
        .map_err(|e| e.to_string())?;
    // ...
}

/// 下载实例 zip 并解压到本地实例目录
pub async fn download_instance(
    host: &str, port: u16, token: &str,
    instance_id: &str, dest_dir: &Path,
    progress: impl Fn(u64, u64), // 已传 / 总量
) -> Result<(), String> {
    let url = format!("http://{host}:{port}/api/instances/{instance_id}");
    let resp = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send().await?;
    // 流式下载 → zip 解压 → 写入 dest_dir
}
```

#### 2.3 命令分发

**`src-tauri/src/commands/dispatch_lan.rs`**（新文件）：

```rust
pub(crate) async fn dispatch(cmd: &str, p: &Value) -> Option<String> {
    match cmd {
        "lan_connect" => { /* connect(host, port, token) */ }
        "lan_list_instances" => { /* 远程实例列表 */ }
        "lan_download_instance" => { /* 下载 + 解压 + 进度 */ }
        _ => None,
    }
}
```

**`src-tauri/src/commands.rs`**：在 `native_invoke` 分发链中加 `dispatch_lan::dispatch`

#### 2.4 Kotlin UI

**新增 `LanSyncPage.kt`**（Compose）：

```
┌─────────────────────────────────────┐
│  ← 局域网同步                       │
│                                     │
│  已连接：PC (192.168.1.100)         │
│                                     │
│  可同步实例：                       │
│  ┌─────────────────────────────┐   │
│  │ ☑ 生存 1.20.1 (Fabric)       │   │
│  │   1.2 GB  [存档][模组][配置]  │   │
│  ├─────────────────────────────┤   │
│  │ ☐ 创造 1.21 (Forge)          │   │
│  │   856 MB                     │   │
│  └─────────────────────────────┘   │
│                                     │
│  [全选] [取消]      [开始同步]      │
│                                     │
│  传输中：生存 1.20.1                │
│  ████████████░░░░  68%  820 MB     │
└─────────────────────────────────────┘
```

- 从 deep link 接收 `host/port/token` → 调 `lan_connect`
- 显示 PC 实例列表 → 用户勾选 → 调 `lan_download_instance`
- 进度通过 `onCoreEvent` 事件推送

**工作量**：~3 天
**风险**：中。安卓存储权限 + 大文件流式写入需仔细处理。

---

### Phase 3：反向同步（手机 → PC）

> Phase 1-2 完成后，反向同步是对称的：手机开 HTTP 服务，PC 连接下载。

#### 安卓端

- `lan_sync.rs` 增加 `start_server`（复用 `axum` 或 raw `TcpListener`）
- Kotlin UI 增加「作为服务端」模式，显示 IP + 端口 + token（或生成二维码图片供 PC 扫）

#### 桌面端

- `lan_sync.rs` 增加 `connect` / `download_instance` 客户端逻辑
- `LanSyncPanel.vue` 增加「连接手机」模式：输入 IP + 端口 + token（或扫手机屏幕上的二维码）

**工作量**：~2 天（复用 Phase 1-2 的代码，对称实现）
**风险**：中。安卓端 HTTP 服务受电池优化 / Doze 影响，需前台服务保活。

---

## 四、文件清单

### 桌面端新增 / 修改

| 文件 | 操作 | 说明 |
|---|---|---|
| `src-tauri/Cargo.toml` | 改 | 加 `axum` + `tower-http` |
| `src-tauri/src/lan_sync.rs` | 新 | HTTP 服务 + 文件打包 + 令牌 |
| `src-tauri/src/commands/lan_sync.rs` | 新 | Tauri 命令 |
| `src-tauri/src/lib.rs` | 改 | 注册模块 + 命令 |
| `src-tauri/src/state.rs` | 改 | AppState 加 `lan_sync: Option<Arc<LanSyncServer>>` |
| `src/components/LanSyncPanel.vue` | 新 | QR 码 + 状态 UI |
| `src/views/SettingsView.vue` | 改 | 加入口 |
| `src/composables/deepLink.ts` | 改 | 加 `lan` 路由 |
| `package.json` | 改 | 加 `qrcode` |
| `src-tauri/tauri.conf.json` | 改 | CSP（如需） |

### 安卓端新增 / 修改

| 文件 | 操作 | 说明 |
|---|---|---|
| `android/app/src/main/AndroidManifest.xml` | 改 | 加 `qookix` scheme intent-filter |
| `ComposeMainActivity.kt` | 改 | `onNewIntent` 解析 deep link |
| `src-tauri/src/lan_sync.rs` | 新 | HTTP 客户端 + 下载 |
| `src-tauri/src/commands/dispatch_lan.rs` | 新 | 命令分发 |
| `src-tauri/src/commands.rs` | 改 | 加 dispatch 分支 |
| `LanSyncPage.kt` | 新 | Compose UI |
| `LanSyncScanner.kt` | 新 | CameraX + ZXing 扫码 |
| `android/app/build.gradle.kts` | 改 | 加 ZXing 依赖 |

---

## 五、估时与风险

| Phase | 内容 | 估时 | 风险 |
|---|---|---|---|
| 0 | Deep Link 统一 | 1 天 | 低 |
| 1 | 桌面端 HTTP 服务 + QR | 3 天 | 低-中（防火墙） |
| 2 | 安卓端客户端 + 扫码 | 3 天 | 中（权限 + 大文件） |
| 3 | 反向同步 | 2 天 | 中（保活） |
| **合计** | | **~9 天** | |

### 关键风险与对策

| 风险 | 对策 |
|---|---|
| Windows 防火墙弹窗 | 首次启动引导用户允许；或文档说明 |
| 安卓 Doze 杀后台服务 | Phase 3 反向同步用前台服务 + 已有 `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` |
| 大文件内存溢出 | 流式下载 + 流式 zip 解压，不一次性 load |
| 局域网 IP 检测失败 | 退化为手动输入 IP |
| 多网卡环境 | 列出所有可用 IP 供用户选择 |
| CSP 阻止前端 fetch | 全部走 Tauri command 中转，前端不直接 fetch HTTP |

---

## 六、复用已有代码

| 已有模块 | 复用方式 |
|---|---|
| `instance_share.rs` | 打包逻辑：`export_pack` 生成 zip → HTTP 服务直接流式返回 |
| `cloud_sync/saves.rs` | `pack_world` / `unpack_to_world`：世界存档打包/解包 |
| `models.rs` | `Instance` / `InstalledContent`：实例元数据序列化 |
| `instances.rs` | 实例列表 / 目录定位 |
| `util.rs` | 通用工具（日志 / 路径处理） |
| `deepLink.ts` | 路由框架：加 `lan` host 分支 |
| Android `native_invoke` | 命令分发：加 `dispatch_lan` 分支 |
| Android `RustBridge` | JNI 通道：复用 `invoke` 调 `lan_*` 命令 |
