# QookiX 跨平台联动方案

> 桌面端（QookiX-Launcher，Tauri 2 + Vue 3 + Rust）与安卓端（QookiX-Launcher-Android，Jetpack Compose + Rust JNI）同属 QookiX 品牌，共享 22 个 Rust 模块，`instance.json` 数据格式已对齐。本文档列出两端可做的联动功能，按可行性与优先级排序。

---

## 一、现状对比

| 维度 | 桌面端 | 安卓端 |
|---|---|---|
| UI 框架 | Vue 3 + naive-ui | Jetpack Compose + Material 3 |
| 后端 | Rust（Tauri 2 IPC） | Rust（JNI 静态库 `libqookix_lib.so`） |
| 共有 Rust 模块 | 22 个（accounts / instances / servers / settings / storage / util …） | 同左 |
| 实例格式 | `Instance` 结构体 → `instance.json` | `MinecraftProfile` 结构体 → `instance.json`（字段对齐 + serde alias 兼容） |
| 云存档 | ✅ GitHub Releases 备份（`cloud_sync_*` 全套） | ❌ |
| Deep Link | ✅ `qookix://` 已注册（launch / skin / settings / instances / news） | ❌ 未注册（`MinecraftProfile.alias` 字段已对齐但 AndroidManifest 缺 intent-filter） |
| 账号 | 微软 + 离线 + 外置登录（Yggdrasil）+ LittleSkin OAuth2 | 微软 + 离线 |
| 开服 | ✅ | ✅ + FRP 内网穿透 |
| 触控层 | — | ✅ 完整虚拟按键 / 摇杆 / 手势 |
| 渲染后端 | 系统 JVM | GL4ES / MobileGlues / Mesa（Zink + Turnip） |
| 更新清单 | `latest.json`（含 `minVersion` 强制更新） | `latest.json`（独立） |
| 服务器列表 | `servers.json`（`ServerConfig`） | `ServerEntry`（name + address + icon） |
| 皮肤管理 | ✅ `skins/` 目录 | ✅ `skins.rs` |

### 共有 Rust 模块清单

```
accounts.rs    crash.rs       download.rs    fsutil.rs      instances.rs
java.rs        launch.rs      lib.rs         main.rs        mcping.rs
mirror.rs      models.rs      modpack.rs     pins.rs        servers.rs
settings.rs    storage.rs     terracotta.rs  translate.rs   updater.rs
util.rs        world_backup.rs
```

### instance.json 字段对齐情况

两端核心字段完全一致：`id`、`name`、`mc_version`、`loader`、`loader_version`、`alias`、`created`、`last_played`、`total_play_time`、`mods`、`resource_packs`、`shaders`、`group`、`max_memory_mb`、`memory_mode`、`jvm_args`、`game_args`、`account_id`、`resolution`、`icon`、`is_symlink`、`source_path`。

安卓端通过 `#[serde(alias = "...")]` 兼容桌面端的驼峰命名（如 `mcVersion` / `lastPlayed` / `gameDir`），实现单向可读。

---

## 二、联动方案

### 🟢 P0 — 马上能做（数据格式已对齐，改动小）

#### 1. 统一 Deep Link `qookix://`

**现状**：桌面端已注册 `qookix://` 协议（`tauri.conf.json` → `deep-link.schemes`），支持 `qookix://launch/<alias>`、`qookix://skin`、`qookix://settings` 等。安卓端 `MinecraftProfile.alias` 字段已对齐（注释写着"与桌面版对齐"），但 `AndroidManifest.xml` 未注册 `qookix` scheme。

**方案**：安卓端 `AndroidManifest.xml` 添加 intent-filter：

```xml
<activity android:name=".compose.ComposeMainActivity">
    <intent-filter>
        <action android:name="android.intent.action.VIEW" />
        <category android:name="android.intent.category.DEFAULT" />
        <category android:name="android.intent.category.BROWSABLE" />
        <data android:scheme="qookix" />
    </intent-filter>
</activity>
```

**效果**：
- 群里 / Discord / 网页发 `qookix://launch/survival`
- 电脑点击 → 桌面端启动对应实例
- 手机点击 → 安卓端启动对应实例
- `qookix://skin/<name>` → 跳转皮肤详情
- `qookix://settings` → 打开设置页

**工作量**：安卓端 ~20 行（intent-filter + URL 解析 + 路由），桌面端无需改动。

**风险**：低。纯增量，不影响现有功能。

---

#### 2. 实例配置互导

**现状**：两端 `instance.json` 字段几乎完全一致。桌面端配好模组组合后，用户需要手动在手机上重新建实例、逐个找模组。

**方案**：导出 / 导入 `.qookix-instance.json` 文件：

```json
{
  "format": "qookix-instance/v1",
  "instance": { /* Instance / MinecraftProfile 公共字段 */ },
  "mods": [ /* InstalledContent 列表 */ ],
  "shaders": [ /* ... */ ],
  "resourcePacks": [ /* ... */ ]
}
```

- **桌面端导出**：实例右键 →「导出配置」→ 生成 `.qookix-instance.json`
- **安卓端导入**：文件管理器打开 `.qookix-instance.json` → 自动建实例 → 下载对应模组
- **反向同理**：安卓端导出 → 桌面端导入

**注意**：导出的是**配置**（版本 + 加载器 + 模组列表），不是游戏文件本身。模组由各端按平台从 Modrinth / CurseForge 重新下载（桌面 x86 / 安卓 arm64）。

**效果**：「电脑上配好模组包 → 发到手机 → 一键导入 → 自动建实例 + 下载模组」

**工作量**：每端 ~100 行（序列化 / 反序列化 + UI 入口 + 导入后自动建实例流程）。

**风险**：低。模组列表是 slug + 版本号，跨平台通用。

---

#### 3. 服务器列表同步

**现状**：桌面端有 `servers.json`（`ServerConfig`，含开服配置），安卓端有 `ServerEntry`（name + address + icon，用于加入服务器）。两端都有多人游戏服务器列表。

**方案**：导出 / 导入服务器收藏列表：

- 桌面端「多人游戏」→ 导出收藏服务器列表 `.qookix-servers.json`
- 安卓端导入 → 服务器列表自动出现
- 反向同理

**效果**：「电脑收藏的服务器，手机上直接能进」

**工作量**：每端 ~50 行（导出 / 导入 + UI 按钮）。

**风险**：低。数据简单（name + address + icon）。

---

#### 4. 局域网直传同步

**现状**：两端实例配置（`instance.json`）格式已对齐，但跨端传输只能靠手动导出文件 + 手动传输（微信 / U 盘 / 蓝牙），或走云端（GitHub Releases）——前者麻烦，后者慢且需授权。

**方案**：局域网内 HTTP 直传，二维码配对：

```
┌─────────────────┐         LAN (HTTP)        ┌──────────────────┐
│   桌面端 (PC)    │◄════════════════════════►│   安卓端 (手机)   │
│                 │                           │                  │
│ ① 开 HTTP 服务  │     ② 扫码配对 (QR)       │ ① 扫二维码        │
│   192.168.1.x   │ ──────────────────────►   │   qookix://sync  │
│   :8765         │                           │   ?host=...      │
│                 │     ③ 选实例 + 传输       │                  │
│ ④ 显示二维码    │ ◄──────────────────────   │ ② 浏览实例列表    │
│   (含令牌)      │                           │ ③ 勾选 + 下载     │
└─────────────────┘                           └──────────────────┘
```

**配对流程**：

1. PC 端打开「设置 → 局域网同步」→ 启动临时 HTTP 服务，绑定局域网 IP
2. PC 端显示二维码，内容为 `qookix://lan-sync/connect?host=192.168.1.100&port=8765&token=a3f8b2c1`
3. 手机扫码 → deep link 唤起 QookiX 安卓端 → 自动连接 PC
4. 手机端看到 PC 上的实例列表 → 勾选要同步的实例 → 一键传输
5. 反向同理：手机端也可开服务，PC 端输入地址连接

**可同步内容**（用户勾选）：

| 内容 | 默认 | 说明 |
|---|---|---|
| 实例配置 `instance.json` | ✅ | 版本 + 加载器 + 模组列表，几 KB |
| 世界存档 `saves/` | 可选 | 可能较大（几十 MB ~ 几 GB），局域网传输快 |
| 模组文件 `mods/` | 可选 | 直接传文件，免去手机端重新从 Modrinth 下载 |
| 资源包 / 光影 | 可选 | 同上 |
| 服务器列表 | 可选 | `servers.json`，几 KB |
| 启动器设置 | 可选 | 选填项（如分辨率 / JVM 参数偏好），路径类设置不同步 |

**HTTP API 设计**：

```
GET  /api/info              → { device: "PC", version: "0.18.0", instances: [...] }
GET  /api/instances         → [ { id, name, mc_version, loader, size_bytes, has_saves } ]
GET  /api/instances/:id/tree → 文件树（目录结构 + 文件大小）
GET  /api/download/:id      → zip 流（实例完整打包）
GET  /api/download/:id/:path → 单文件流
POST /api/upload            → 接收 zip 流（反向同步）
GET  /api/servers           → 服务器列表 JSON
POST /api/servers           → 接收服务器列表
```

所有请求需 `Authorization: Bearer <token>` 头，token 为配对时二维码中的随机令牌，仅本次会话有效。

**安全设计**：

- HTTP 服务绑定局域网 IP（非 `0.0.0.0`），仅同局域网可访问
- 一次性令牌（32 字节随机），配对时生成，服务关闭即失效
- 不持久化任何配对信息，关闭面板即停止服务
- 传输不加密（局域网内无必要），但令牌防止陌生设备连接

**效果**：
- 「电脑上配好实例 → 手机扫码 → 10 秒同步完成」
- 局域网速度：100 Mbps 约 12 MB/s，1 GB 存档约 85 秒
- 无需 GitHub 授权、无需外网、无需手动传文件

**工作量**：
- 桌面端 ~400 行（HTTP 服务 + 二维码生成 + 文件打包 + UI 面板）
- 安卓端 ~300 行（HTTP 客户端 + 扫码 + 文件解包 + deep link 路由 + UI）
- 两端可复用 `instances.rs` / `storage.rs` / `util.rs` 等共有模块

**风险**：低-中。
- 需处理防火墙（Windows 首次启动可能弹允许提示）
- 需处理安卓端网络权限 + SAF 存储权限
- 大文件传输需流式处理，避免内存溢出
- mDNS 自动发现可作为增强（避免手动输入 IP），但二维码已足够

**与 Deep Link 的关系**：本功能依赖 P0-1（Deep Link 统一）。二维码内容是 `qookix://lan-sync/connect?...`，手机扫码后由 deep link 自动唤起 QookiX 安卓端并跳转到同步页面。若 Deep Link 尚未实现，可退化为手机端手动输入 IP + 端口 + 令牌。

---

### 🟡 P1 — 需要开发但不难

#### 5. 云存档互通

**现状**：桌面端已有 `cloud_sync_*` 全套 API（GitHub Releases 备份世界存档，含上传 / 拉取 / 列表 / 恢复 / 自动备份 / 保留数量管理）。安卓端无云存档。

**方案**：安卓端接入同一套 GitHub Releases 备份机制：

- 安卓端 `SavesTab` 增加「云备份」入口
- 复用桌面端的 `cloud_sync_upload` / `cloud_sync_restore` / `cloud_sync_list_all` 逻辑
- 安卓端世界存档路径不同（`/sdcard/Android/data/.../instances/<id>/saves/`），但备份格式统一为 zip

**效果**：
- 电脑上玩到一半 → 推送存档到 GitHub Releases
- 手机端拉取存档 → 继续玩
- 反之亦然
- 自动备份策略两端共享配置

**工作量**：安卓端 ~200 行（GitHub API 调用 + zip 打包 / 解包 + UI）。

**风险**：中。需处理 GitHub Token 安全存储（安卓 Keystore）。

---

#### 6. 账号便捷登录

**现状**：两端都走微软设备码流程（Device Code Flow），用户需要分别在两端完成登录。令牌仅存本机。

**方案**：扫码授权（用户主动操作，不自动同步）：

- 桌面端「账号管理」→「扫码登录其他设备」→ 显示二维码（含临时授权码）
- 安卓端扫码 → 获取令牌 → 存入本机
- 令牌有时限，过期后各端独立刷新

**备选方案**（更简单）：导出 / 导入账号配置文件（`.qookix-account.json`），含刷新令牌。用户手动传输（AirDrop / 蓝牙 / 文件）。

**效果**：「电脑上登录的微软账号，手机上不用再走一遍设备码流程」

**工作量**：扫码方案 ~300 行（临时授权服务 + 二维码 + 安卓扫码）；导出导入方案 ~80 行。

**风险**：中。令牌安全传输需谨慎，文档需明确告知用户风险。

---

#### 7. 皮肤同步

**现状**：两端都有皮肤管理（桌面 `skins/` 目录，安卓 `skins.rs`）。皮肤文件是 PNG，格式统一。

**方案**：
- 桌面端「皮肤中心」→ 换皮肤 → 勾选「同步到其他设备」→ 推送到 GitHub Releases（复用云存档基础设施）
- 安卓端拉取 → 自动应用
- 或更简单：导出 / 导入 `.png` 文件

**效果**：「换皮肤两端同时生效」

**工作量**：~100 行（复用 cloud_sync 基础设施）。

**风险**：低。

---

### 🔴 P2 — 需要较大投入但品牌价值高

#### 8. 跨平台整合包

**现状**：两端都支持 Modrinth / CurseForge 整合包导入。但同一整合包在桌面和手机上需要分别导入，且某些模组可能不跨平台。

**方案**：定义「QookiX 跨平台整合包」格式（`.qookix-modpack`）：

```json
{
  "format": "qookix-modpack/v1",
  "name": "我的整合包",
  "mcVersion": "1.20.1",
  "loader": { "type": "fabric", "version": "0.15.11" },
  "mods": {
    "common": [ /* 跨平台通用模组 */ ],
    "desktop": [ /* 仅桌面端模组（如 OptiFine 替代） */ ],
    "android": [ /* 仅安卓端模组（如性能优化） */ ]
  },
  "shaders": { "common": [], "desktop": [], "android": [] }
}
```

- 桌面端导入 → 装 `common` + `desktop` 模组
- 安卓端导入 → 装 `common` + `android` 模组
- 用户只需一次下载，两端通用

**效果**：「一个整合包，电脑手机都能用」

**工作量**：~400 行（格式定义 + 导入逻辑 + 导出 UI + 模组平台兼容性检测）。

**风险**：中。需维护模组跨平台兼容性数据。

---

#### 9. 局域网自动发现联机

**现状**：桌面端能开服，安卓端能开服 + FRP 内网穿透。两端都有 `mcping.rs`（服务器 ping）。但用户需要手动输入服务器地址。

**方案**：mDNS / SSDP 局域网服务发现：

- 开服端广播 `_minecraft._tcp` mDNS 服务
- 客户端自动发现局域网内的 QookiX 服务器
- 服务器列表显示「局域网」分区，一键加入
- 不在同一局域网 → 安卓端 FRP 穿透 → 生成加入链接 → 分享给电脑端

**效果**：「电脑开服，手机自动发现，点一下就进」

**工作量**：~500 行（mDNS 广播 / 发现 + UI + FRP 链接生成）。

**风险**：中。mDNS 在某些网络环境可能被屏蔽。

---

#### 10. 统一更新清单

**现状**：两端各自维护 `latest.json`，各自触发 CI 构建。强制更新 `minVersion` 各自独立。

**方案**：合并为单一多平台清单：

```json
{
  "version": "0.18.0",
  "minVersion": "0.16.0",
  "notes": "QookiX 0.18.0 更新",
  "platforms": {
    "windows-x86_64": { "url": "...", "signature": "..." },
    "linux-x86_64":   { "url": "...", "signature": "..." },
    "macos-aarch64":  { "url": "...", "signature": "..." },
    "macos-x86_64":   { "url": "...", "signature": "..." },
    "android-arm64":  { "url": "...", "signature": "..." }
  }
}
```

- 一个 `minVersion` 同时强制两端更新
- 品牌统一发版：一个 Release 同时发桌面 + 安卓
- CI 统一触发：推一个 tag → 构建所有平台 → 发布统一清单

**效果**：「QookiX 0.18.0 更新」——不分桌面和安卓，品牌统一感知。

**工作量**：~300 行（CI 工作流合并 + 清单格式 + 两端 updater 适配）。

**风险**：中。需协调两端发版节奏。

---

### 💡 P3 — 品牌层面（长期）

#### 11. 统一视觉语言

两端 UI 框架不同（Vue vs Compose），但可以对齐：
- 配色方案（主色 / 强调色 / 背景色）
- 圆角 / 间距 / 字号阶梯
- 动画曲线与时长
- 图标集（两端都用 Lucide / Feather）
- 术语统一（「实例」/「投影工坊」/「联机房间」等）

**效果**：用户从电脑切到手机，零学习成本。

---

#### 12. 统一发版与 Release Notes

- 两端 CHANGELOG 格式已一致（修复 → 新增 → 优化）
- 统一品牌发版页面：一个 GitHub Release 同时包含桌面 + 安卓产物
- Release Notes 统一撰写，用户看到的是「QookiX 0.18.0 更新」而非两个独立项目

---

#### 13. 跨端游玩时长统计

两端都记录 `total_play_time`（实例级累计游玩时长）。同步到统一统计：
- 云端聚合各设备时长
- 用户看到「你在 QookiX 上总共玩了 1234 小时」——跨设备
- 可选：周报 / 月报推送

---

## 三、优先级总览

| 优先级 | 联动 | 改动量 | 品牌感知 | 推荐理由 |
|---|---|---|---|---|
| 🥇 P0 | Deep Link 统一 | 小 | 强 | 改动最小（安卓加 intent-filter），效果最直观 |
| 🥈 P0 | 局域网直传同步 | 中 | 强 | 局域网直传速度快，二维码配对体验好，不依赖外网 |
| 🥉 P0 | 实例配置互导 | 小 | 强 | 数据格式已对齐，用户痛点明确 |
| P0 | 服务器列表同步 | 小 | 中 | 数据简单，实现快 |
| P1 | 云存档互通 | 中 | 强 | 桌面端已有基础设施，"跨设备继续玩"是最强卖点 |
| P1 | 账号便捷登录 | 中 | 中 | 减少重复操作，但安全需谨慎 |
| P1 | 皮肤同步 | 小 | 中 | 复用云存档基础设施 |
| P2 | 跨平台整合包 | 大 | 强 | "一个包两端用"，品牌差异化 |
| P2 | 局域网自动发现联机 | 大 | 强 | "电脑开服手机自动进"，联机体验 |
| P2 | 统一更新清单 | 中 | 中 | 品牌统一发版 |
| P3 | 统一视觉语言 | 持续 | 强 | 零学习成本，长期品牌建设 |
| P3 | 统一发版 | 小 | 中 | 品牌统一感知 |
| P3 | 跨端时长统计 | 中 | 弱 | 锦上添花 |

---

## 四、建议实施路径

```
Phase 1（快速见效）
  └─ Deep Link 统一          ← 1 天
  └─ 局域网直传同步          ← 4 天（依赖 Deep Link）
  └─ 实例配置互导            ← 2 天
  └─ 服务器列表同步          ← 1 天

Phase 2（核心卖点）
  └─ 云存档互通              ← 3 天
  └─ 皮肤同步                ← 1 天
  └─ 账号便捷登录            ← 2 天

Phase 3（品牌差异化）
  └─ 跨平台整合包            ← 5 天
  └─ 局域网自动发现联机      ← 5 天
  └─ 统一更新清单            ← 3 天

Phase 4（长期建设）
  └─ 统一视觉语言            ← 持续
  └─ 统一发版                ← 持续
  └─ 跨端时长统计            ← 2 天
```

---

## 五、技术约束与注意事项

1. **模组跨平台兼容**：并非所有模组都支持安卓（如依赖原生库的模组）。整合包互导时需检测兼容性并提示用户。
2. **令牌安全**：账号令牌跨端传输需使用安全通道（扫码 / 加密文件），不可明文传输。
3. **文件路径差异**：桌面端路径用 `/` 或 `\`，安卓端用 `/sdcard/...`。`instance.json` 中的 `game_dir` / `java_path` 等路径字段不应跨端直接复制，应由各端在导入时重新计算。
4. **数据格式版本化**：所有导出文件应包含 `format` 版本字段，便于后续格式升级时的兼容处理。
5. **CI 协调**：统一更新清单需要两端 CI 产出同一 manifest，需协调构建时机和产物上传顺序。
6. **安卓存储权限**：安卓端导入 / 导出文件需处理 SAF（Storage Access Framework）权限，不能直接访问任意路径。
