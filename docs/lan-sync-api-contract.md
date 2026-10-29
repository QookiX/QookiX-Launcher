# 局域网同步 — API 契约（安卓端对接用）

> 桌面端已实现下载 + 上传。安卓端照此文档对接即可，不需要看桌面端代码。

## 配对流程

1. 桌面端用户在「设置 → 局域网同步」点「启动服务」
2. 桌面端 Rust 启动 HTTP 服务，绑定局域网 IP
3. 前端渲染 QR 码，内容为 deep link URL
4. 手机扫码 → `qookix://lan/pair?host=<ip>&port=<port>&token=<token>`
5. 安卓端解析 URL → 连接 HTTP 服务 → 浏览实例 → 下载

## QR 码内容格式

```
qookix://lan/pair?host=192.168.1.100&port=8765&token=a3f8b2c1d4e5f6a7b8c9d0e1f2a3b4c5
```

| 参数 | 类型 | 说明 |
|---|---|---|
| `host` | string | 桌面端局域网 IP |
| `port` | u16 | HTTP 服务端口（系统分配的空闲端口） |
| `token` | string | 32 字节 hex 令牌（一次性，服务关闭即失效） |

## HTTP API

所有请求需带 `Authorization: Bearer <token>` 头。无 token 或 token 错误返回 `401 Unauthorized`。

### `GET /api/info`

获取设备信息。

**响应**：
```json
{
  "device": "MY-PC-NAME",
  "version": "0.17.0",
  "instance_count": 5,
  "features": ["download", "upload"]
}
```

> `features` 字段用于能力协商：手机端只有看到 `"upload"` 才显示「发送到电脑」入口。
```

### `GET /api/instances`

获取实例列表。

**响应**：
```json
[
  {
    "id": "a1b2c3d4",
    "name": "生存 1.20.1",
    "mc_version": "1.20.1",
    "loader": "Fabric",
    "loader_version": "0.15.11",
    "size_bytes": 123456789,
    "has_saves": true,
    "has_mods": true
  }
]
```

### `GET /api/instances/:id`

下载实例 zip 包。流式返回。

**响应**：
- `Content-Type: application/zip`
- `Content-Disposition: attachment`
- Body: zip 文件字节流

**zip 内容**：实例目录下所有文件，包括：
- `qookix.json` — 实例配置
- `mods/` — 模组文件
- `saves/` — 世界存档
- `resourcepacks/` — 资源包
- `shaderpacks/` — 光影
- `config/` — 模组配置
- `options.txt` — 游戏设置
- `servers.dat` — 服务器列表

**不包含**：游戏本体（libraries / assets / versions），导入端需自行安装游戏本体。

**路径穿越防护**：`id` 含 `/`、`\` 或 `..` 时返回 `400 Bad Request`。

### `GET /api/servers`

获取服务器列表 JSON。

**响应**：`servers.json` 文件内容（`Content-Type: application/json`）。不存在时返回 `404`。

### `POST /api/instances/upload` — 手机上传实例到电脑

```
POST /api/instances/upload?name=<urlenc>&mc_version=1.21.1&loader=fabric&loader_version=0.16.0
Authorization: Bearer <token>
Content-Type: application/zip
Content-Length: <zip 字节数>

<zip 流>
```

**Query 参数**：

| 参数 | 必填 | 说明 |
|---|---|---|
| `name` | ✅ | 实例名（URL 编码，允许中文/空格） |
| `mc_version` | ✅ | Minecraft 版本 |
| `loader` | ✅ | `vanilla` / `fabric` / `quilt` / `forge` / `neoforge` |
| `loader_version` | vanilla 时可空 | 加载器版本 |

**Body**：zip 流（`Content-Type: application/zip`）。zip 内含 `instance.json` + `mods/` + `saves/` + `config/` 等实例文件。

**成功响应**（201）：
```json
{ "ok": true, "instance_id": "a1b2c3d4e5f6...", "name": "月落花野" }
```

**失败响应**：

| 状态码 | 含义 |
|---|---|
| 401 | token 无效 |
| 409 | 同名实例已存在（`{ "ok": false, "error": "同名实例已存在: xxx" }`） |
| 413 | 超过 10 GB 限制 |
| 500 | 解压/落盘失败（`{ "ok": false, "error": "..." }`） |

**桌面端处理流程**：
1. 同名检查 → 409
2. 收集 body（限制 10 GB）→ 413
3. 生成新 UUID，解压到 `<instances_dir>/<uuid>/`
4. 转换 `instance.json`（安卓格式）→ `qookix.json`（桌面格式）
5. 删除 `instance.json`，返回新实例 ID

## 安卓端实现要点

### 1. AndroidManifest.xml — 注册 deep link

```xml
<activity android:name=".compose.ComposeMainActivity" ...>
    <!-- 现有 intent-filter -->
    <intent-filter>
        <action android:name="android.intent.action.VIEW" />
        <category android:name="android.intent.category.DEFAULT" />
        <category android:name="android.intent.category.BROWSABLE" />
        <data android:scheme="qookix" />
    </intent-filter>
</activity>
```

### 2. ComposeMainActivity.kt — 解析 URI

```kotlin
override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    intent.data?.let { uri ->
        if (uri.scheme == "qookix" && uri.host == "lan") {
            val host = uri.getQueryParameter("host") ?: return
            val port = uri.getQueryParameter("port")?.toIntOrNull() ?: return
            val token = uri.getQueryParameter("token") ?: return
            // 跳转到 LanSyncPage，传入 host/port/token
            pageHost.navigate("lan-sync", LanSyncParams(host, port, token))
        }
    }
}
```

### 3. Rust HTTP 客户端（复用已有 reqwest）

```rust
// 连接 + 获取实例列表
async fn lan_connect(host: &str, port: u16, token: &str) -> Result<DeviceInfo, String> {
    let resp = reqwest::Client::new()
        .get(format!("http://{host}:{port}/api/info"))
        .header("Authorization", format!("Bearer {token}"))
        .send().await
        .map_err(|e| e.to_string())?;
    resp.json::<DeviceInfo>().await.map_err(|e| e.to_string())
}

// 下载实例 zip 并解压
async fn lan_download_instance(
    host: &str, port: u16, token: &str, instance_id: &str,
    dest_dir: &Path,
) -> Result<(), String> {
    let resp = reqwest::Client::new()
        .get(format!("http://{host}:{port}/api/instances/{instance_id}"))
        .header("Authorization", format!("Bearer {token}"))
        .send().await?;
    let bytes = resp.bytes().await?;
    // 解压 zip 到 dest_dir
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor)?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let name = file.name().to_string();
        let outpath = dest_dir.join(&name);
        if file.is_dir() {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() { std::fs::create_dir_all(p)?; }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }
    Ok(())
}
```

### 4. 命令分发

在 `commands.rs` 的 `native_invoke` 中加新命令：

```rust
"lan_connect" => { /* lan_connect(host, port, token) */ }
"lan_list_instances" => { /* GET /api/instances */ }
"lan_download_instance" => { /* GET /api/instances/:id → 解压 */ }
```

### 5. Kotlin UI

```
┌─────────────────────────────────────┐
│  ← 局域网同步                       │
│                                     │
│  已连接：MY-PC-NAME (192.168.1.100) │
│                                     │
│  可同步实例：                       │
│  ┌─────────────────────────────┐   │
│  │ ☑ 生存 1.20.1 (Fabric)       │   │
│  │   117 MB  [存档][模组]       │   │
│  ├─────────────────────────────┤   │
│  │ ☐ 创造 1.21 (Forge)          │   │
│  │   856 MB                     │   │
│  └─────────────────────────────┘   │
│                                     │
│  [全选] [取消]      [开始同步]      │
│                                     │
│  传输中：生存 1.20.1                │
│  ████████████░░░░  68%  80 MB      │
└─────────────────────────────────────┘
```
