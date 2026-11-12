# 房间整合包 P2P 同步 —— 详细设计

> 状态：**设计稿 v0.1，未实现**。本文只描述方案，不含代码改动。
> 前置讨论结论（已确认）：
> 1. 服务器带宽极小，**文件流量一个字节都不能经过服务端**；服务端只承载几十字节信令。
> 2. 整合包大小、内容**完全由用户决定**，设计不得假设任何体积上限或内容构成。
> 3. 默认策略：**只同步 mod + config**；资源包 / 光影 / 截图存档默认不勾，由用户主动选择。
> 4. 先出设计文档，确认后再分阶段开工。

---

## 1. 目标与非目标

**目标**

- 房主开联机房间时，可附带一份「环境清单」；加入者点一次「同步环境」即可补齐 mod、游戏版本、可选资源包与 config。
- 流量走房主 ↔ 加入者的 P2P 通道，不经 QookiX 服务端。
- 在点下「同步」之前，双方都能看到**要传什么、多大、大概多久**，并能随时取消。
- 环境一致时零传输。

**非目标**

- 不做公共文件 CDN、不做服务端对象存储、不做 TURN 中继（会重新引入服务端带宽）。
- 不做聊天、不做存档同步（存档默认不进清单）。
- 不解决「房主下线后新成员补齐」——这是 P2P 的固有限制，明确接受。
- 不实现 BitTorrent 式多源：已补齐成员当第二源留到二期评估。

---

## 2. 已验证的技术前提（本机实测，非推测）

陶瓦（Terracotta 0.4.2）内嵌 EasyTier `v2.5.0-terracotta.2`。从 `%LOCALAPPDATA%\Temp\terracotta\<时间戳>\application.log` 读到它开房时的真实启动参数：

```
--network-name scaffolding-mc-7RPT-NQBB  --network-secret JRB2-0RRL
-p tcp://public.easytier.top:11010  -p tcp://public2.easytier.cn:54321 ...
--no-tun  --p2p-only  --compression=zstd --enable-kcp-proxy
--ipv4 10.144.144.1  --tcp-whitelist=13448  --tcp-whitelist=48762
rpc=34450
```

| 结论 | 依据 | 对设计的影响 |
|---|---|---|
| `--no-tun`：不建虚拟网卡、不需管理员权限 | 实测 `host-ok` 后系统网卡/IP 列表无变化 | 启动器可安全起独立节点，不影响用户网络设置 |
| 虚拟 IP 只在 EasyTier 进程内部 | `--ipv4 10.144.144.1` 但系统路由表无此地址 | 必须用 `port-forward` 映射成本地端口 |
| 陶瓦白名单只放行认证端口 + MC 端口 | `--tcp-whitelist=13448,48762` | **不能借陶瓦那张网开 HTTP**，必须起独立网络 |
| `easytier-cli whitelist set-tcp` / `port-forward add` 支持运行时动态改 | `easytier-cli --help` 实测 | 加端口不用重启节点 |
| 独立 `--no-tun` 双节点可 P2P 直连 | 同机双实例 `peer list` 显示 `cost=p2p / tunnel=tcp / 0.26ms` | 通道成立 |
| **任意 TCP 文件流量可穿过隧道** | A 起 HTTP 服务，B 执行 `port-forward add tcp 127.0.0.1:18081 10.144.145.1:18080`，`curl` 得 `200` 且拿到完整内容 | 方案可行性已证实 |

**唯一未验证项**：跨公网 NAT 打洞。本机实验用的是局域网 IP 直连（`-p tcp://192.168.1.114:11012`），没走公共节点。但陶瓦实际联机时用 `--p2p-only` + 公共节点已在真实网络跑通，该能力视为已被现实验证；上线前仍需一次双机实测。

### 2.1 实测更正（2026-11-12，EasyTier 2.5.0）

按上面的理解去实现，本机双节点**连不上**。逐项排查后有三条必须改：

| # | 原文的理解 | 实测事实 |
|---|---|---|
| 1 | `--p2p-only` = 只要 P2P、不要中继 | **反了**。官方帮助原文：「禁用 P2P 通信，只通过 `--peers` 指定的节点转发数据包」。带上它节点之间不会直连，**必须去掉** |
| 2 | 用 `--rpc-portal` ？ | 参数是 `-r / --rpc-portal`，默认 `0`（先试 15888）。**同机两节点必须错开端口**，否则第二个起不来 |
| 3 | peers 可以写 `127.0.0.1:port` | `--no-tun` 下连接回环地址直接 `WSAEADDRNOTAVAIL(10049)`。**必须用真实网卡 IP**（`tcp://192.168.1.114:11013` 可以，`tcp://127.0.0.1:11013` 不行） |

另外：`public.easytier.top` 在本机 DNS 解析失败（`no records found for public.easytier.cn`），
公共 rendezvous 不可用。所以当前实现**不依赖公共节点**，改用「房主 `-l` 监听 + 客人 `-p` 直连」。

验证结果（本机双节点，回环局域网）：

```
B peer list: 10.144.171.1/24  cost=p2p  lat=0.13ms  tunnel=tcp   ← 直连成立
port-forward add tcp 127.0.0.1:18101 → 10.144.171.1:18099        ← 规则生效
curl http://127.0.0.1:18101/hello.txt → 200 + 完整内容            ← 隧道承载 HTTP 成功
```

**依赖来源**：`easytier-core.exe` / `easytier-cli.exe` 由陶瓦每次运行解压到 `%LOCALAPPDATA%\Temp\terracotta\<ts>\embedded-easytier\`。设计上先按「复用陶瓦释放的副本」实现，同时预留「启动器自带一份」的开关，避免陶瓦版本变动导致路径失效。

---

## 3. 总体架构：清单层与文件层分离

```
                    ┌──────────── QookiX 服务端 ────────────┐
                    │  只存「房间 → 清单/连接信息」几十字节   │
                    │  零文件流量，零新增存储                │
                    └───────────────────────────────────────┘
                          ▲ 房间元数据            │ 读取
                          │                      ▼
┌──── 房主 ────────────────────────┐        ┌──── 加入者 ──────────────────────┐
│ 1. 扫磁盘生成清单（几 KB）        │        │ 1. 拉清单 + 本地算指纹比对        │
│ 2. 起独立 EasyTier 节点           │        │ 2. 起 EasyTier 节点加入同一网络   │
│ 3. 起单文件 HTTP 服务             │◀──P2P─▶│ 3. port-forward 映射本地端口      │
│ 4. 房间元数据发布连接信息         │  文件   │ 4. 分片下载缺失文件 → 导入实例    │
└──────────────────────────────────┘        └──────────────────────────────────┘
```

**核心原则**：清单永远只有几 KB，走服务端；文件内容大小不定，走 P2P。这样无论用户整合包是 50MB 还是 8GB，服务端负担恒定。

**在线来源的 mod 不进 P2P**：清单里带 `provider + project_id + version_id` 的条目，加入者直接走 Modrinth / CurseForge 重下（复用 `import_instance_pack` 已有的 `PendingDownload` 链路）。房主上行只承担「加入者缺的、且无在线来源」的文件。

---

## 4. 数据契约

### 4.1 房间元数据（后端）

`rooms.modpack` 是 ≤120 字自由文本，**放不下清单**。新增独立字段：

```
rooms.pack_json   TEXT NOT NULL DEFAULT ''   -- 清单 JSON 压缩后（上限 64 KB）
rooms.pack_net    TEXT NOT NULL DEFAULT ''   -- 连接信息（≤255 字）
rooms.pack_hash   TEXT NOT NULL DEFAULT ''   -- 内容指纹，16 hex
rooms.pack_size   INTEGER NOT NULL DEFAULT 0 -- 需 P2P 传输的字节数（展示用）
```

`pack_net` 形如：

```
qkx1|<network-name>|<network-secret>|<host-virtual-ip>|<port>|<token>
```

- `token`：一次性、房间级，`randomHex(16)`，房主生成
- **公开大厅的房间元数据人人可读**，因此 `network-secret` 不能当作安全凭据（见 §9）
- 房间关闭 / 心跳超时 / 房主主动停止 → 四个字段一起清空

不新增表、不新增文件接口。`POST /rooms`、`heartbeat`、`close` 增加可选字段；`roomView` 回吐这些字段。

### 4.2 清单 JSON

以**磁盘为准**生成（不依赖 `qookix.json` 的登记记录，用户整合包里大量手工文件没有登记）：

```json
{
  "v": 1,
  "mc_version": "1.20.1",
  "loader": "fabric",
  "loader_version": "0.15.11",
  "files": [
    {
      "kind": "mod | resourcepack | shaderpack | config",
      "name": "sodium-fabric-1.20.1.jar",
      "size": 812345,
      "sha1": "…",
      "source": { "provider": "modrinth", "project_id": "…", "version_id": "…" },
      "optional": false
    }
  ]
}
```

- `optional: true` 用于资源包 / 光影 / 截图存档 —— 对应「默认不勾」
- `source` 为 `null` 表示无在线来源，必须走 P2P
- 清单 gzip 后存 `pack_json`；超过 64 KB 时只存指纹 + 让加入者通过 P2P 拉完整清单（降级路径）

### 4.3 指纹

```
fingerprint = SHA256( mc_version | loader | loader_version |
                      sort(每个文件的 sha1 + size + kind) ) 前 16 位 hex
```

必须包含**文件内容哈希**，只列文件名会把「同 mod 换版本」误判为一致。

大包首次算哈希会慢，缓存到实例目录 `pack_fingerprint.json`：按 `size + mtime` 判断文件是否变动，仅对变动文件重算 SHA1（复用 `keybinds_lang.rs` 的 `scan_fingerprint` 思路）。指纹在后台算，不阻塞开房。

---

## 5. 状态机

**房主**

```
IDLE → SCANNING(扫磁盘/算指纹，可后台) → READY(清单已生成)
     → PUBLISHING(起 EasyTier + HTTP，写房间元数据) → SERVING
     → STOPPED(关房 / 心跳超时 / 手动停止 / 启动器退出)
```

任何异常都回 `STOPPED` 并清空 `pack_*`；节点与 HTTP 服务随状态销毁，不留后台进程。

**加入者**

```
IDLE → FETCH_MANIFEST → COMPARE(本地比对)
     → ESTIMATE(展示差异与预计耗时，等待用户确认)
     → JOIN_NET(起节点 + port-forward) → TRANSFER(分片下载)
     → IMPORT(复用 import_instance_pack) → DONE
     → CANCELLED / FAILED(可重试，已下载分片保留)
```

---

## 6. 关键流程

**开房**：房主勾选「共享环境」→ 后台扫清单算指纹 → 起独立 EasyTier 网络 `qkx-<room>`（不同 name/secret，`--no-tun`，白名单含自己 HTTP 端口）→ 起单文件 HTTP 服务 → 写 `pack_json / pack_net / pack_hash / pack_size`。

**同步**：加入者点「同步环境」→ 拉清单 → 本地按 `sha1 + size` 比对 → 分类展示：

```
已一致 168 个文件
在线可重下 31 个（走 Modrinth/CF，不占房主）
需从房主拉取 15 个 · 共 480 MB · 按当前上行约 3 分钟
  [只同步 mod + config]（默认）  [加上资源包/光影]  [取消]
```

确认后才起 EasyTier 节点并 `port-forward add`；下载完按清单写回实例目录，在线条目交给 `PendingDownload` 补齐，游戏版本交给 `install_game`。

**取消 / 失败**：随时可中断，已下载分片保留；节点与 port-forward 规则一并撤销。房主下线 → 明确提示「房主已离线，暂时无法补齐」，不静默重试。

---

## 7. 默认策略（已确认）

| 内容 | 默认 | 理由 |
|---|---|---|
| mod（在线来源） | ✅ 走 CDN 重下 | 不占房主带宽 |
| mod（无在线来源） | ✅ P2P | 不补齐会拒连 / 崩 |
| config | ✅ P2P | 体积小，不补齐可能异常 |
| 游戏版本 / 加载器 | ✅ 自动安装 | `install_game` 现成 |
| 资源包 / 光影 | ⬜ 默认不勾 | 体积最不可控，且只影响画面 |
| 截图 / 存档 | ⬜ 默认不勾 | 与联机无关 |

不写死：记住用户上次选择；房主可设「只挂清单、不允许传文件」或完全关闭共享。

---

## 8. 传输层（应对「大小由用户决定」）

- **分片 + 断点续传**：按文件切块（建议 1 MB），带 `Range`；断线重连从缺块继续。
- **默认限速**：可配置，默认留余量给 MC 服（建议 8 MB/s 起步，房主可调），避免上行打满导致游戏卡。
- **流式**：不整包载入内存；下载直写临时文件，全部校验通过后再落入实例目录。
- **可预估、可确认**：传输前展示总字节与预计耗时；超过用户设定阈值时二次确认，而不是设计期硬编码上限。
- **进度与取消**：房主侧显示「谁在拉、传到哪」（复用 `lan_sync` 已有的进度推送机制）。
- **校验**：每块与整文件都验 SHA1，失败只重传该块。

---

## 9. 安全

| 风险 | 处置 |
|---|---|
| 公开大厅 `pack_net` 人人可读，`network-secret` 等于公开 | 不把网络层当 ACL：叠加一次性 `token`（HTTP 服务校验）、只暴露**单个已打包文件**、节点随房间关闭即销毁 |
| `lan_sync` 现有 `GET /api/instances/:id` 返回整个实例目录 zip | **不得复用**。新写一个只返回清单 / 单文件的精简服务 |
| 任意用户可加入同一 EasyTier 网 | 网络只用于文件传输；节点 `--no-tun` 不接管系统路由；白名单只开 HTTP 端口 |
| 恶意房主下发路径穿越 / 超大清单 | 清单名称白名单校验、禁 `..` 与绝对路径（沿用 `import_pack` 的 zip-slip 防护）、`pack_json` 64 KB 上限、按 uid 限流 |
| 传输内容夹带任意文件 | 只接受清单内登记的文件名，导入前校验 SHA1 与扩展名白名单 |

---

## 10. 失败降级

| 情况 | 行为 |
|---|---|
| 指纹一致 | 不建节点、不传文件，直接提示「环境一致」 |
| P2P 打洞失败 | 提示「无法直连房主」，退回手动：房主导出 `.qkxinst`，加入者走现有 `import_instance_pack` |
| 陶瓦未运行 / 找不到 `easytier-core.exe` | 同上降级，不阻塞联机 |
| 清单超 64 KB | 服务端只存指纹与连接信息，完整清单走 P2P 拉取 |
| 房主中途下线 | 提示离线，保留已下载分片 |
| 在线 mod 下载失败 | 逐条标失败，不阻断其余文件，允许重试 |

---

## 11. 改动清单（供确认后实施）

**后端 `QookiX-Service-Backend`**

- `apiv1.go`：`rooms` 加 `pack_json / pack_net / pack_hash / pack_size` 四列（`ensureBusinessColumn` 补列）
- `rooms.go`：`roomUpsertRequest` / `roomView` / `roomColumns` / `scanRoom` 同步；`close` 与心跳超时时清空；`GET /rooms`、`/rooms/public` 回吐
- 限流：按 uid 的 `packLimiters`，防刷大清单
- `API_V1.md` 补字段说明

**客户端 `QookiX-Launcher`**

- 新增 `src-tauri/src/room_pack/`：清单扫描与指纹缓存、EasyTier 进程管理（复用陶瓦释放的 exe）、单文件 HTTP 服务、分片下载器
- `qookix/rooms.rs`：`share_room` 增加可选 pack 字段；`poller.rs` 心跳带上 pack 状态；退出清理节点
- `api.ts` / `stores/qookix.ts`：新增同步相关命令与状态
- UI：房主卡片「共享环境」开关、房间卡片环境信息与「同步环境」按钮、差异预估弹窗、进度条
- `import_instance_pack` 增加「按清单合并到现有实例」模式（现有实现是创建新实例）
- i18n：`qookix.zh.ts` / `qookix.en.ts` 新增 key（**用户已把这两个 fragment 挂进 `zh-CN.ts` / `en-US.ts`，`vue-tsc` 已通过**）

---

## 12. 分阶段实施

| 阶段 | 内容 | 可独立验收 |
|---|---|---|
| P0 | 清单生成 + 指纹缓存 + 房间元数据四字段 + 房间卡片显示「环境一致 / 不一致」 | ✅ 零 P2P 流量，就能解决「进房才发现版本不对」 |
| P1 | 独立 EasyTier 节点 + 单文件 HTTP + port-forward + 分片下载（默认 mod/config） | ✅ 单机可测，双机验证跨网 |
| P2 | 在线来源重下合并、资源包可选勾选、限速 / 断点 / 取消、进度 UI | ✅ 完整体验 |
| P3 | 失败降级到 `.qkxinst` 手动导入、房主策略设置 | ✅ 健壮性 |

P0 单独上线就有价值，且风险最低；建议按 P0 → P1 推进，每阶段单独验收。

---

## 13. 未决问题

1. **跨公网 NAT 打洞成功率**：需一次双机实测（`--p2p-only` 无中继，对称 NAT / CG-NAT 下可能失败）。若失败，评估去掉 `--p2p-only` 让 EasyTier 走公共节点中继——流量仍不经过 QookiX 服务端，但依赖第三方节点。
2. **EasyTier 二进制来源**：优先复用陶瓦释放的副本，但需处理「陶瓦未安装 / 版本升级 / 临时目录被清理」；是否自带一份进安装包待定（约 20 MB）。
3. **房主在线限制**已确认接受，但 UI 必须明确表达，不能让人以为后台会自动补齐。
4. **已补齐成员当第二源**留到二期，取决于 P1/P2 的实际传输耗时反馈。
