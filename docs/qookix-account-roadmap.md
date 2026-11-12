# QookiX 账号功能路线图

> 状态：**设计稿 v0.1，未实现**。本文只描述方案与改动点，不含代码改动。
> 前置已确认：QookiX 账号定位为**可选账号**，唯一职责是「好友 + 联机房间目录」，不接管 Minecraft 登录。
> 关联文档：
> - `docs/room-pack-sync-design.md` —— 房间整合包 P2P 同步（**环境清单的唯一权威定义**，本文不重复设计）
> - `docs/cross-platform-synergy.md` —— 桌面端 / 安卓端联动

---

## 0. 结论摘要

| 期 | 功能 | 价值 | 成本 | 依赖 |
|---|---|---|---|---|
| Phase 0 | 后端底座（清单字段 / KV 表 / presence 扩展） | — | 中 | ✅ **已完成并上线（2026-11-12）** |
| Phase 2a | **联机环境预检**（只读比对，不传文件） | ★★★★★ | 中 | Phase 0 |
| Phase 1 | 房间活跃度展示 | ★★★☆☆ | 极低 | 无（纯前端） |
| Phase 3 | 配置云同步（设置 + 收藏） | ★★★★☆ | 低 | Phase 0 |
| Phase 4a | 好友 Activity（在玩什么） | ★★★★☆ | 低 | Phase 0 |
| Phase 2b | 环境补齐（P2P 传文件 / 在线重下） | ★★★★☆ | 高 | 2a + room-pack-sync-design |
| Phase 4b | 服务器目录 | ★★★☆☆ | 高 | Phase 0 |

**核心判断**：这个项目同时掌握「启动器 + 服务端 + 联机通道」，是目前唯一能把**环境一致性**和**打洞联机**打通的项目。竞品要么只管打洞（陶瓦 / SakuraFrp），要么只管环境（HMCL / PCL2 / Modrinth App），没人同时管。

**调整说明**：原提案里的「房间延迟」被砍掉。实测陶瓦 `/state/` 的 `profiles` 只有 `kind / machine_id / name / vendor`，**没有 ping 数据**；访客端也拿不到房主公网地址去测速。硬做只能编数字，改为诚实的「活跃度」。

---

## 1. 现状盘点

### 1.1 服务端已有能力（`QookiX-Service-Backend`）

| 能力 | 实现位置 | 备注 |
|---|---|---|
| 账号 / JWT / 刷新 / 找回密码 | `accounts.go`、`apiv1.go:749+` | 6 条公开路径 |
| 好友 / 申请 / 拉黑 / 搜索 | `friends.go` | `relation` 已修正 `accepted` 悬挂问题 |
| 在线状态 | `accounts.go:677` `handleAPIPresence` | **请求体为空**，Redis `presence:uid:<uid>` 存 `"1"`，TTL 90s |
| 房间目录 / 心跳 / 大厅 | `rooms.go` | `play_type`、`version`、`min_players` 筛选已上线 |
| 邀请 / 通知 | `rooms.go`、`friends.go:574+` | 通知只推新增，客户端需定期全量对账 |
| 举报 / 下架 / 封禁 / 审计 | `rooms.go`、`admin_api.go` | 管理端已支持举报行内「下架房间 / 封禁房主」 |

### 1.2 客户端已有可复用资产

| 资产 | 位置 | 对本次规划的意义 |
|---|---|---|
| `InstalledContent`（含 `source / project_id / version_id / mod_id / version / enabled`） | `src-tauri/src/models.rs:239-269` | **环境清单可直接从实例 JSON 生成，不用解析 jar** |
| 安装时写回来源 | `modrinth.rs:414-418`、`curseforge.rs:561-564` | 在线来源可精确重下 |
| `parse_mod_jar`（fabric / quilt / mods.toml / neoforge） | `src-tauri/src/util.rs:752` | manual 来源补 modid / version |
| `identify_manual_mods`（散列反查 Modrinth） | `src-tauri/src/instance_share.rs:478` | 手动 mod 有机会回填 project_id |
| 实例导出 / 导入（`PendingDownload` 在线重下） | `instance_share.rs:665 / :860` | 2b 的兜底链路 |
| `Settings` 字段分组 | `src-tauri/src/models.rs:34-137` | 云同步白名单依据 |
| 收藏夹（localStorage 快照） | `src/stores/favorites.ts:15-34` | 云同步可顺带覆盖 |
| `servers.dat` 读取（NBT / json） | `src-tauri/src/commands/multiplayer.rs:15` | 4b 的数据源；**无写回实现** |

### 1.3 服务端尚缺

- `rooms` **没有**清单字段（`modpack` 是 ≤120 字自由文本，放不下）
- **没有**通用用户 KV 表
- **没有**服务器目录表（`rooms` 是唯一目录表）
- presence **没有请求体结构**，加字段需要新建 struct
- `reports.kind` 是自由 TEXT，但管理端 JOIN 只按 `'room'` 分支

---

## 2. Phase 0：后端底座

一次性把 schema 做完，后续四期都只改业务逻辑，避免反复部署。

### 2.1 `rooms` 扩展

字段名**沿用** `room-pack-sync-design.md:87` 的约定，只补一个列表用的计数字段：

```sql
ALTER TABLE rooms ADD COLUMN pack_json  TEXT    NOT NULL DEFAULT '';  -- 清单 JSON（gzip 后 ≤64KB）
ALTER TABLE rooms ADD COLUMN pack_hash  TEXT    NOT NULL DEFAULT '';  -- 清单指纹，快速判等
ALTER TABLE rooms ADD COLUMN pack_size  INTEGER NOT NULL DEFAULT 0;   -- 文件总字节数
ALTER TABLE rooms ADD COLUMN mods_count INTEGER NOT NULL DEFAULT 0;   -- 仅用于列表展示
```

> **关键约束**：`pack_json` **不进列表接口**（单条 64KB × 20 条会打爆限流与带宽）。
> 列表只返回 `mods_count` + `pack_hash`；完整清单通过 `GET /api/v1/rooms/detail` 单独取。

### 2.2 新表 `user_settings`

```sql
CREATE TABLE IF NOT EXISTS user_settings (
    uid        INTEGER NOT NULL,
    key        TEXT    NOT NULL,
    value_json TEXT    NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(uid, key)
);
```

- 单行上限 64KB（应用层校验）
- 目标用途：`launcher`（启动器设置白名单字段）、`favorites`
- 冲突策略：只做**乐观锁**（带上 `updated_at`，服务端比对新旧，冲突返回 409 让客户端选择）

### 2.3 presence 扩展

现状：`accounts.go:677` 不读 body，Redis 值写死 `"1"`。

改为：

```go
type presenceBody struct {
    Instance  string `json:"instance,omitempty"`   // 实例名，≤64
    McVersion string `json:"mc_version,omitempty"` // ≤32
    Loader    string `json:"loader,omitempty"`     // forge/fabric/... ≤32
    Server    string `json:"server,omitempty"`     // 所在服务器地址，≤128，用户可关
}
```

- Redis 值从 `"1"` 改为 JSON 字符串；`onlineMap` / `ttlAlive` 只判 `EXISTS`，**不受影响**
- `/friends`、`/rooms`、`/notifications` 返回的用户对象里追加 `activity`（仅在线且有值时下发）
- 隐私：客户端设置项 `share_activity`（默认开），关闭后一律不传 `Server`

### 2.4 新接口（Phase 0 一并注册）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/me/settings` | 取全部 KV（或 `?keys=a,b`） |
| PUT | `/api/v1/me/settings` | 覆盖写，带 `updated_at` 乐观锁 |
| DELETE | `/api/v1/me/settings?key=` | 删除单项 |

注册需同步改：`apiv1.go:749+` 路由、`authmw.go:318` `allAPIPaths`、`authmw.go:402` `forceAuth`（按需）。限流默认走 `apiLimiters`（2/s，burst 120）。

### 2.5 改动清单

**后端**
- `apiv1.go:549-565` 建表加列；`apiv1.go:620-645` 加 `ensureBusinessColumn` 调用
- `apiv1.go:420` `initBusinessTables` 加 `user_settings`
- `rooms.go:24-40 / :47 / :49-55 / :140-157 / :159-183 / :195-203 / :232-255 / :379-381 / :390-391`
- 列表查询：`rooms.go:296-301`、`rooms.go:609-614`（只加 `mods_count` / `pack_hash`）
- 详情：`roomViewOf` 增加 `pack`（仅 active 且可见时）
- `admin_api.go:521-526`（管理端列表只加 `mods_count`）
- 新文件 `sync_settings.go`（KV CRUD）+ `accounts.go:677` presence 改造

**客户端**（Phase 0 只做数据层，UI 留到各期）
- `src-tauri/src/qookix/commands.rs` 加 `qookix_get_settings / qookix_put_settings`
- `src-tauri/src/qookix/poller.rs` 心跳体带上 activity
- `src/types.ts`、`src/api.ts`、`src/stores/qookix.ts` 同步字段

### 2.6 验收

- `go build` + `go test` 通过；本地起服务，`ALTER TABLE` 在 2GB 库上秒级完成
- `GET /rooms/detail` 能拿到 `pack`，`GET /rooms` 只返回 `mods_count`
- 心跳带 activity 后，`/friends` 返回里能看到 `activity.instance`
- 旧客户端（不带 activity）调心跳不报错

### 2.7 实现落地（2026-11-12 已完成）

实际接口形态与上面的草案有两处**简化**，以本节为准：

1. **指纹由服务端算**。客户端只传 `pack_json` + `mods_count`；
   `pack_hash`（sha256 前 8 字节 hex）与 `pack_size` 由服务端计算，客户端无法伪造。
2. **`pack_size` 只在详情接口返回**。列表查询为了减少列宽只取 `pack_hash` / `mods_count`。

上线验证（生产库）：

```
rooms 新增列: pack_json / pack_hash / pack_size / mods_count
新表: user_settings
```

本地端到端验收（全部通过）：

| # | 检查 | 结果 |
|---|---|---|
| 1 | 注册、加好友 | ✅ |
| 2 | 上报带清单的房间 | `pack_hash=38d2f36b…`、`mods_count=3` |
| 3 | 心跳带 activity | ✅ |
| 4 | 好友列表带 `activity.instance / loader` | ✅ |
| 5 | 房间列表有 `mods_count` / `pack_hash`，**无 `pack`** | ✅（关键约束生效） |
| 6 | 详情接口返回完整 `pack`（3 个 mod） | ✅ |
| 7 | KV：PUT → GET → 409 冲突 → 413 超限 → DELETE | ✅ |

---

## 3. Phase 2a：联机环境预检（只读）

> 与 `room-pack-sync-design.md` 的关系：**清单格式、指纹算法、P2P 传输全部沿用该设计**。本阶段只做它 §5 的 `IDLE → FETCH_MANIFEST → COMPARE` 段，**不做文件传输**。

### 3.1 为什么先做「只读预检」

联机失败里绝大多数不是打洞失败，而是**双方环境不一致**（版本 / 加载器 / mod 版本不同），进房就崩，然后互相甩锅。P2P 传文件是重工程（EasyTier 子网、单文件 HTTP 服务、分片下载），而**「告诉用户差在哪」只需要清单**，能吃掉 80% 的困惑。

### 3.2 数据流

```
房主：开房 → 勾选「共享环境」
      → 扫实例 qookix.json 的 InstalledContent（无需解析 jar）
      → 生成清单 JSON（≤64KB）→ gzip → POST /rooms {pack_json, pack_hash, pack_size, mods_count}

访客：点「加入」
      → GET /rooms/detail 拿 pack
      → 与当前选中实例比对（mod_id + version + mc_version + loader）
      → 三档结果：
         ① 完全一致           → 直接加入
         ② 仅缺 mod / 版本不符 → 弹差异卡片，列出缺什么，提供「安装」入口（2b 才自动补齐）
         ③ 版本或加载器不符    → 红色警告，默认不允许加入（可强制）
```

### 3.3 清单生成规则

- 依据 `InstalledContent`：只取 `enabled == true` 的 mod
- 每项：`{mod_id, name, version, size, sha1?, provider, project_id, version_id}`
  - **有 `project_id`** → 记在线来源，加入者可直接重下，不走 P2P（对齐 `room-pack-sync-design.md:76`）
  - **`source == manual`** → 无 `project_id`，先跑 `identify_manual_mods` 尝试回填；仍失败则标记 `manual`，2b 只能走 P2P 传文件
- 超 64KB：只上报 `pack_hash` + `mods_count`，完整清单走 P2P 拉取（降级路径，对齐 §4.2）

### 3.4 改动清单

**客户端**
- 新模块 `src-tauri/src/room_pack/manifest.rs`：从 `Instance` 生成清单 + 指纹
- `qookix_share_room` 增加可选 `pack` 参数（由前端在「共享环境」勾选后传入）
- 新命令 `qookix_compare_pack(room_id, instance_id)` → 返回差异结构
- `RoomDirectoryPanel.vue` 加入前的预检卡片
- 差异渲染：`缺 3 个 / 多 2 个 / 版本不符` + 每项展开

**后端**
- `roomView` 加 `pack`（仅详情接口）；列表加 `mods_count`

### 3.5 验收

- 两台机器、同实例 → 点加入零提示直接进
- 故意删一个 mod → 对方看到「缺失 N 个」并列出名字
- 版本不同 → 禁止加入并说明原因
- 清单 > 64KB 时降级为「只能看到数量，无法比对」

### 3.6 风险

- 手工 mod（无 project_id）无法自动补齐 → 文案必须诚实：「需要房主传给你」或「自行安装」
- 清单体积：500 个 mod 的整合包 JSON 约 60-80KB → 必须有 64KB 上限与降级

### 3.7 实现落地（2026-11-12 已实现，待双机实测）

**关键实测结论：陶瓦不暴露游戏目录 / mod 清单。**
逐个探测过它的本地 HTTP 接口，只有 `/state/*`（scanning / guesting / ide）能用，
`/mods`、`/files`、`/game`、`/dir`、`/config` 全是 404。

也就是说**陶瓦房间真正跑的 Minecraft 是它自己管的**，启动器拿不到那份 mod 清单。
所以本阶段的清单描述的是**房主的启动器实例**，对陶瓦房间来说是「参照物」而不是「真相」——
UI 上必须如实说明，不能让人以为比对通过就一定不崩。

已实现：

| 层 | 内容 |
|---|---|
| Rust | `src-tauri/src/room_pack/mod.rs`：清单生成（只取启用的 mod，标识优先用 `mod_id`，缺失时退回文件名）+ 三类差异比对 + 2 条命令 + 2 个单测 |
| Rust | `qookix_share_room` 支持 `pack` / `mods_count`；换房重报时清单一起带上（否则会被覆盖抹掉） |
| 前端 | 弹窗统一成「分享给好友 / 公开到大厅」两种入口，都带「附带整合包环境」开关 + 实例选择 |
| 前端 | 房间卡片显示「房主附带 N 个模组」；点「加入」先比对，有差异弹卡片（缺哪些 / 哪些版本不同 / 版本与加载器是否一致），可「仍然加入」或先不加入 |
| 后端 | Phase 0 已就绪（`pack_json` / `pack_hash` / `mods_count`） |

**已补（2b 快路径，2026-11-12）**：预检弹窗里可以直接「一键安装缺失的 N 个模组」——
只装清单里带 `provider + project_id + version_id` 的条目，走现有 `install_content` 管线（Modrinth / CurseForge）。
装完自动重跑一次比对，环境齐了就提示并关窗。
`manual` 来源（手动拖进来的 jar）没有在线来源，只能提示「向房主要文件」，等 P2P 那套（`room-pack-sync-design.md` §8）。

**待补**：P2P 传文件补齐 manual 模组。

---

## 4. Phase 1：房间活跃度

**纯前端**，无后端改动。

- 用 `roomView.last_heartbeat` / `expires_at` / `member_count` 渲染：
  - `3 分钟前活跃 · 2 人`
  - 超过 `ROOM_TTL_SEC` 一半（60s）标黄「可能已结束」，超过 TTL 标灰
- 大厅与好友房间共用同一套卡片渲染
- 顺手把 `play_type` 标签、版本、整合包排成统一的一行 tags

**验收**：房主退出 / 心跳超时后，卡片 30 秒内（轮询周期）变灰。

> **已实现（2026-11-12）**：房间卡片多一枚 `{n} 分钟前活跃` 标签（`utils/qookix.ts::agoText`，
> 组件内 30 秒 tick 一次刷新），非 `active` 房间该标签压暗到 0.6。
> 状态徽标仍由服务端 `roomEffectiveStatus` 计算，两者配合：徽标说「在不在」，标签说「多久没动静」。

---

## 5. Phase 3：配置云同步

### 5.1 同步范围（白名单，不做整包）

| 组 | 字段（`models.rs`） |
|---|---|
| 内存 / 参数 | `max_memory_mb`、`min_memory_mb`、`memory_mode`、`jvm_args`、`game_args` |
| 下载 / 网络 | `download_threads`、`download_chunk_threads`、`proxy_mode`、`proxy`、`mirror`、`mirror_custom`、`content_mirror`、`content_mirror_custom` |
| 外观 | `theme`、`theme_color`、`background_image`、`background_blur`、`background_dim`、`glass_blur`、`show_news`、`hidden_nav_items` |
| 行为 | `language`、`close_behavior`、`auto_launch`、`keep_open`、`auto_update`、`update_source`、`translate_provider`、`translate_api_base`、`translate_api_model`、`body_translate_auto` |
| 收藏 | `favorites`（`stores/favorites.ts` 的 `FavoriteItem[]`） |

### 5.2 明确排除

- 本机相关：`data_dir`、`java_path`、`selected_account`、`onboarding_completed`、`dismissed_update_version`、`close_behavior_prompted`
- 敏感：`curseforge_api_key`、`shimu_client_secret`、`littleskin_client_secret`、`translate_api_key`（**永不上云**）
- 实例骨架（B 方案）本阶段不做

### 5.3 交互

- 设置 → 账号 → QookiX 卡片下增加「配置云同步」区块
- **手动**上传 / 下载，不做自动覆盖
- 下载前提示「将覆盖本地 N 项设置，建议先上传备份」
- 冲突：服务端 `updated_at` 比本地记录新 → 提示「云端有更新的版本（x 分钟前）」

### 5.4 改动清单

- 新文件 `src-tauri/src/qookix/sync_settings.rs`：白名单序列化 / 应用
- `SettingsView.vue` 同步区块
- `favorites.ts` 增加 `exportSnapshot()` / `importSnapshot()`，云端值并入本地

**验收**：A 机上传 → B 机下载 → 主题、镜像、内存参数、收藏夹一致；密钥字段保持各自本地值。

> **已实现（2026-11-12）**：
> - `src-tauri/src/qookix/sync.rs`：**30 个字段的白名单** + `whitelist_drops_secrets_and_local_paths` 单测
>   （校验 `*_api_key` / `*_client_secret` / `data_dir` / `java_path` / `background_image` 都进不来）
> - 4 条命令：`qookix_sync_upload` / `qookix_sync_download` / `qookix_kv_get` / `qookix_kv_put`
> - 下载侧**再过滤一次白名单**：服务端存了什么都不能越权改本机设置
> - 下载走 `settings::update_settings` 复用既有 patch 逻辑（含落盘），前端 `settings.load()` 后界面立即生效
> - 收藏走通用 KV（key=`favorites`），并且是**合并**不是覆盖——收藏只增不删，覆盖会吞掉另一台机器刚加的条目
> - 明确排除 `background_image`：那是本机绝对路径，同步到另一台就是破图
> - UI：设置 → 账号 → QookiX 行下方「配置云同步」，上传/恢复两个按钮，恢复前有确认弹窗

---

## 6. Phase 4a：好友 Activity

- presence 心跳体带 `activity`（Phase 0 已就绪）
- 好友页每行副标题：`1.20.1 Forge · 在玩 XX 整合包` / `在 XX 服务器`
- 房间卡片显示房主当前实例名
- 设置项 `share_activity`（默认开），关闭后不上报 `Instance` / `Server`

**验收**：A 启动游戏 → B 的好友页 45s 内出现「在玩 …」；A 关闭开关 → 60s 内消失。

> **已实现（2026-11-12）**：
> - 心跳带上 activity：`poller.rs::current_activity` 从 `state.game_pids` 取正在运行的实例，
>   报 `instance / mc_version / loader`；**不报 `server`**（本机私密信息，当前也没有可靠来源）。
> - 开关：`Settings.share_activity`（默认开，`#[serde(default = "default_true")]`，老配置缺键即视为开），
>   设置 → 账号 → QookiX 行下方有开关，关掉后心跳回到空 body。
> - 展示：好友行第二行副标题 `1.20.1 fabric · 在玩 XX`（`FriendsPanel::activityLabel`）；
>   房间卡片的房主名后面跟当前实例名。

---

## 7. Phase 4b：服务器目录（建议押后）

### 7.1 为什么押后

- 成本最高：新表 + 新路由 + `reports.kind='server'` 分支 + 管理端改造
- 冷启动最难：没有服务器数据时就是个空页面
- 数据来源受限：现有只有 `servers.dat` 读取（`commands/multiplayer.rs:15`），**没有写回**

### 7.2 草案

```sql
CREATE TABLE IF NOT EXISTS servers_dir (
    id          TEXT PRIMARY KEY,
    owner_uid   INTEGER NOT NULL,
    address     TEXT NOT NULL,        -- host:port
    title       TEXT NOT NULL DEFAULT '',
    game_version TEXT NOT NULL DEFAULT '',
    loader      TEXT NOT NULL DEFAULT '',
    online_mode INTEGER NOT NULL DEFAULT 1,   -- 正版验证
    play_type   TEXT NOT NULL DEFAULT '',
    motd        TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL DEFAULT 'active',
    report_count INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
```

- 复用房间目录的筛选（`play_type` / `version` / 在线人数）
- 地址由用户主动分享才上传；不上报即不公开
- 举报扩展 `kind='server'` → 管理端按 kind 分别 JOIN

---

## 8. API 增补汇总

| 方法 | 路径 | 期 | 说明 |
|---|---|---|---|
| GET | `/api/v1/me/settings` | 0 | 取配置 KV |
| PUT | `/api/v1/me/settings` | 0 | 写配置 KV（乐观锁） |
| DELETE | `/api/v1/me/settings` | 0 | 删 KV |
| POST | `/api/v1/rooms` | 0 | body 增加 `pack_json / pack_hash / pack_size / mods_count` |
| GET | `/api/v1/rooms/detail` | 0 | 返回增加 `pack` |
| GET | `/api/v1/rooms`、`/rooms/public` | 0 | 返回增加 `mods_count`、`pack_hash`、owner `activity` |
| POST | `/api/v1/presence/heartbeat` | 0 | body 增加 `activity` 字段 |
| POST | `/api/v1/rooms`（清单） | 2a | 客户端生成清单并上报 |
| * | `/api/v1/servers*` | 4b | 服务器目录（草案） |

---

## 9. 兼容与降级

| 场景 | 策略 |
|---|---|
| 旧客户端心跳无 body | 服务端保持可选字段，缺省视为无 activity |
| 清单 > 64KB | 只存指纹与数量，完整清单走 P2P（已有设计 §4.2） |
| KV 单条 > 64KB | 拒绝并返回 `payload_too_large`，客户端提示「收藏过多，请分批」 |
| 服务端未升级 | 客户端遇到 404 一律降级为本地行为，不阻塞主流程 |
| `mods_count` 为 0 的旧房间 | 卡片不显示 mod 数量，不显示预检入口 |

---

## 10. 不做的事

- **聊天**：做不过 QQ/Discord，且要背内容审核责任
- **动态流 / 评论区**：冷启动做不起来，最后是没人看的社交坟场
- **成就系统**：投入产出比低
- **服务端存文件**：清单只走服务端，文件一律 P2P（`room-pack-sync-design.md` 已定）

---

## 10.5 带宽预算（已评估，结论：不构成瓶颈）

服务器出口 **20 Mbps（≈2.5 MB/s，216 GB/天）**，按单用户每天 28 MB（现有 4 个轮询接口的总量）估算：

| 日活 | 单日出口 | 平均占用 | 峰值（×3） | 占 20 Mbps |
|---|---|---|---|---|
| 30 | 0.84 GB | 0.08 Mbps | 0.24 Mbps | 1% |
| 100 | 2.8 GB | 0.26 Mbps | 0.8 Mbps | 4% |
| 500 | 14 GB | 1.3 Mbps | 3.9 Mbps | 20% |
| 2000 | 56 GB | 5.2 Mbps | 15.6 Mbps | 78% |

- **云同步本身可忽略**：设置 2–4 KB + 收藏 8–15 KB，手动触发，一个月十几次也就几百 KB
- **P2P 传文件完全不经服务端**（`room-pack-sync-design.md` 已定），20 Mbps 只承担信令
- 真要压流量，大头上是轮询（4 个接口 12,000 次/天），但**要到几百日活才会开始吃紧**

**决定：轮询瘦身（合并 `/sync`、按页面可见性门控、自适应退避）标记为延后，日活过 300 再评估。**
唯一必须守住的是「`pack_json` 不进列表接口」——单条 64 KB × 20 条会一次吃掉 1.3 MB 上行，
那已经不是带宽问题，而是「打开联机页卡三秒」的问题。

---

## 11. 排期建议

```
Phase 0（后端底座）       ✅ 已完成并上线
Phase 2a（预检）          1.5 天   ← 下一步，价值最高
Phase 1（活跃度）         0.5 天
Phase 3（云同步）         1 天
Phase 4a（Activity）      0.5 天   ← 后端已就绪（presence/activity 已上线），只剩 UI
───── 以上为第一轮可发布范围 ─────
Phase 2b（补齐 P2P）      3-5 天   ← 按 room-pack-sync-design §12 走
Phase 4b（服务器目录）     2-3 天
Phase 0.5（轮询瘦身）      延后     ← 日活过 300 再评估，见 §10.5
```

---

## 12. 已确认的决策

| # | 问题 | 结论 |
|---|---|---|
| 1 | `pack_json` 是否沿用 `room-pack-sync-design.md` 的清单格式 | **是**，字段名与格式完全复用，避免二次迁移 |
| 2 | 云同步首发范围 | **只做「设置白名单字段 + 收藏」**，实例骨架留到 B 方案 |
| 3 | 服务器目录 | **押后到第一轮之外**（Phase 4b） |
| 4 | 轮询瘦身（Phase 0.5） | **延后**，日活过 300 再评估 |
| 5 | 「房间延迟」指标 | **砍掉**，改为止心跳活跃度（无可靠数据源） |
