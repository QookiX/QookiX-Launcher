import type { Component } from "vue";

export type Loader = "vanilla" | "fabric" | "quilt" | "forge" | "neoforge";

/** 右键菜单项；`sep` 为真时渲染为分隔线，其余字段忽略 */
export interface ContextMenuItem {
  key: string;
  label?: string;
  icon?: Component;
  /** 右侧显示的快捷键提示，仅作展示 */
  shortcut?: string;
  danger?: boolean;
  disabled?: boolean;
  sep?: boolean;
  action?: () => void;
}

/**
 * 前端可见的设置（后端 `settings::frontend_view` 的投影）。
 * API Key 这类敏感字段不会下发，只有 `*_set` / `*_hint`。
 */
export interface Settings {
  data_dir: string;
  java_path: string | null;
  max_memory_mb: number;
  min_memory_mb: number;
  memory_mode: string;
  jvm_args: string;
  game_args: string;
  download_threads: number;
  download_chunk_threads: number;
  /** CurseForge API Key 是否已配置（Key 原文不下发前端） */
  curseforge_api_key_set: boolean;
  /** 已保存的 CurseForge Key 尾号（如 `····ab12`），未配置为空串 */
  curseforge_api_key_hint: string;
  theme: string;
  theme_color: string;
  /** 界面语言："zh-CN" | "en-US" */
  language: string;
  /** 关闭窗口行为："ask"（每次询问，默认）| "minimize" | "quit" */
  close_behavior: string;
  auto_launch: boolean;
  keep_open: boolean;
  ms_client_id: string;
  selected_account: string | null;
  /** 下载代理模式："system"（系统代理）| "direct"（直连）| "custom"（自定义） */
  proxy_mode: string;
  /** 自定义代理地址（proxy_mode === "custom" 时生效） */
  proxy: string | null;
  /** 下载镜像源 id："official" | "bmclapi" | "custom" */
  mirror: string;
  /** 自定义镜像根地址（mirror === "custom" 时生效） */
  mirror_custom: string;
  /** 内容 API 镜像 id："official"（默认）| "mcim" | "custom"，只影响发现页 / 内容详情的查询接口 */
  content_mirror: string;
  /** 自定义内容 API 镜像根地址（content_mirror === "custom" 时生效） */
  content_mirror_custom: string;
  /**
   * 背景类型：`gradient`（渐变底）| `image`（自定义图片）| `scene`（实时渲染的动态场景）。
   * 空字符串 = 自动：有 background_image 用图片，否则渐变（兼容旧配置）。
   */
  background_mode: string;
  /**
   * 动态场景 id（`scene` 模式下生效），见 `src/components/background/scenes.ts`。
   * 空字符串 = 默认场景，兼容没有这个字段的旧配置。
   */
  background_scene: string;
  /**
   * 动态场景画质：`low`（流畅 / 省电）| `medium`（均衡）| `high`（高清）。
   * 空字符串 = 默认档，见 `src/components/background/quality.ts`。
   */
  background_quality: string;
  background_image: string | null;
  background_blur: number;
  background_dim: number;
  glass_blur: number;
  /** 新闻页面与侧边栏新闻入口是否显示（默认 true） */
  show_news: boolean;
  /** 用户在设置页选择隐藏的侧边栏导航项 name 列表 */
  hidden_nav_items: string[];
  dismissed_update_version: string | null;
  auto_update: boolean;
  /** 应用自更新源："bucket"（对象存储，默认） | "github"（GitHub Releases 官方源） */
  update_source: "bucket" | "github";
  /** 内容描述翻译服务："default"（自建翻译服务） | "custom"（OpenAI 兼容接口） */
  translate_provider: string;
  /** 打开内容详情时自动加载正文译文 */
  body_translate_auto: boolean;
  /** 是否向好友显示「我正在玩什么」（QookiX 心跳上报），默认开 */
  share_activity?: boolean;
  /** 自定义翻译 API 的 OpenAI 兼容地址 */
  translate_api_base: string;
  /** 自定义翻译 Key 是否已配置（Key 原文不下发前端） */
  translate_api_key_set: boolean;
  /** 已保存的翻译 Key 尾号（如 `····ab12`） */
  translate_api_key_hint: string;
  /** 自定义翻译使用的模型名 */
  translate_api_model: string;
  /** 新手向导是否已完成（首次启动为 false，完成后置 true） */
  onboarding_completed: boolean;
}

/** 下载镜像源预设 */
export interface MirrorPreset {
  id: string;
  label: string;
  /** 镜像站根地址，官方源为空串 */
  base: string;
  desc: string;
}

export interface MirrorTestResult {
  ok: boolean;
  ms: number;
  url: string;
}

export interface JavaInfo {
  path: string;
  version: string;
  major: number;
  vendor: string;
  arch: string;
}

export interface StorageCategory {
  key: string;
  label: string;
  size: number;
  files: number;
}

export interface InstanceStorage {
  id: string;
  name: string;
  size: number;
  files: number;
}

export interface StorageStats {
  categories: StorageCategory[];
  instances: InstanceStorage[];
  servers: InstanceStorage[];
  total: number;
  instance_count: number;
  server_count: number;
  updated_at: number;
  cached: boolean;
}

export interface CacheClearResult {
  freed: number;
  java_download_in_use: boolean;
  errors: string[];
}

export interface Instance {
  id: string;
  name: string;
  mc_version: string;
  loader: Loader;
  loader_version: string | null;
  created: number;
  last_played: number | null;
  /** 累计游玩时长（秒），由后端在游戏进程退出时累加 */
  total_play_time: number;
  installed: boolean;
  icon: string | null;
  /** 实例别名（qookix://launch/<alias> 协议启动用，全局唯一） */
  alias?: string | null;
  max_memory_mb: number | null;
  memory_mode: string | null;
  jvm_args: string | null;
  game_args: string | null;
  java_path: string | null;
  account_id: string | null;
  resolution: [number, number] | null;
  mods: InstalledContent[];
  resource_packs: InstalledContent[];
  shaders: InstalledContent[];
  is_symlink?: boolean;
  source_path?: string | null;
  /** 所属分组 id，null / undefined 表示未分组 */
  group?: string | null;
}

/** 实例分组（持久化在 instance_groups.json） */
export interface InstanceGroup {
  id: string;
  name: string;
  color: string | null;
  created: number;
}

export type Account =
  | { type: "offline"; uuid: string; username: string; created: number }
  | {
  type: "microsoft";
  uuid: string;
  username: string;
  created: number;
  msa_expires_at: number;
  }
  | {
  type: "yggdrasil";
  uuid: string;
  username: string;
  created: number;
  /** 皮肤站 Yggdrasil API root */
  server: string;
  /** 展示名（如 LittleSkin） */
  server_name: string;
  };

// ---------------------------------------------------------------------------
// 拾木空间（www.mcimg.com）第三方接入：OAuth 授权登录 + 开放 API
// ---------------------------------------------------------------------------

/** 后端接入配置（client_secret 不会下发到前端） */
export interface ShimuConfig {
  host: string;
  base: string;
  authorizeUrl: string;
  /** 已登记的回调地址（qookix://oauth/callback） */
  redirectUri: string;
  scope: string;
  clientId: string;
  /** client_id 是否已配置 */
  configured: boolean;
  /** client_secret 是否已配置（只返回真假，不回传密钥） */
  secretConfigured: boolean;
  /** 开发构建才为 true：只有开发/联调时提供「接入设置」入口 */
  devMode: boolean;
  /** 下载的投影文件存放目录 */
  downloadDir: string;
}

/** LittleSkin OAuth2（Yggdrasil Connect）接入配置 */
export interface YggOauthConfig {
  /** client_id 是否已配置（决定是否露出「OAuth2 登录」入口） */
  configured: boolean;
  clientId: string;
  /** 回调地址 qookix://oauth/littleskin/callback */
  redirectUri: string;
  authorizeUrl: string;
  scope: string;
  /** client_secret 是否已配置（只返回真假，不回传密钥） */
  secretConfigured: boolean;
  /** 开发构建才为 true */
  devMode: boolean;
}

export interface YggOauthResult {
  authorizationId: string;
  profiles: { id: string; name: string }[];
  server: string;
  serverName: string;
  links: { homepage: string; register: string };
}

/** `getInfo` 返回的用户资料（字段以服务端为准，未给出的为可选） */
export interface ShimuUser {
  userId?: number;
  userName?: string;
  nickName?: string;
  avatar?: string;
  signature?: string;
  /** 积分 */
  score?: number;
  /** 余额（元） */
  balance?: number;
  /** 0 非会员 / 1 普通会员 / 2 高级会员 */
  memberLevel?: number;
  memberName?: string;
  /** null 表示长期 */
  vipExpireTime?: string | null;
  /**
   * 我的世界 UUID（去连字符小写）。
   * ⚠️ 仅作资料展示/识别用：拾木空间账号不能当启动账号，也不得以它当作
   * 通过 Minecraft 正版验证的身份（不得用于绕过正版验证的外置登录）。
   */
  mcUuid?: string;
  /** 皮肤 / 披风相对路径（拼 HOST 访问） */
  skin?: string;
  cape?: string;
  title?: string;
  exp?: number;
  level?: number;
}

export interface ShimuStatus {
  loggedIn: boolean;
  /** 本地判定已过期（access_token 有效期 3 天且无 refresh_token） */
  expired: boolean;
  expiresAt: number;
  user: ShimuUser | null;
}

/** 帖子对象（列表 / 详情共用，付费字段为字符串 "0"/"1"） */
export interface ShimuPost {
  postId: number;
  title?: string;
  /** 富文本 HTML；付费帖未购买时为空 */
  content?: string;
  summary?: string;
  cover?: string;
  images?: string;
  userId?: number;
  authorName?: string;
  authorAvatar?: string;
  authorMemberLevel?: number;
  categoryId?: number;
  categoryName?: string;
  tagNames?: string | null;
  isFeatured?: string;
  isTop?: string;
  viewCount?: number;
  likeCount?: number;
  commentCount?: number;
  collectCount?: number;
  /** 0 免费 / 1 付费 */
  isPaid?: string;
  /** 0 余额 / 1 积分（免费帖为 null） */
  payType?: string | null;
  payAmount?: number | null;
  /** 0 内容付费 / 1 文件下载付费 */
  payTarget?: string;
  liked?: boolean;
  collected?: boolean;
  purchased?: boolean;
  commented?: boolean;
  createTime?: string;
}

/** 分页返回（`total` + `rows`） */
export interface ShimuPage<T> {
  code: number;
  msg?: string;
  total: number;
  rows: T[];
}

/** 站点板块（分类）：`/api/forum/category/list` 的 data 项 */
export interface ShimuCategory {
  categoryId: number;
  name: string;
  /** 该板块帖子数 */
  postCount?: number;
  parentId?: number;
}

export interface ShimuPostQuery {
  categoryId?: number;
  /**
   * 多板块合并查询（站点只支持单个 categoryId）。
   * 投影工坊「投影相关」默认传「投影 / 红石 / 生电 / 建筑」四个板块。
   */
  categoryIds?: number[];
  versionId?: number;
  tagId?: number;
  userId?: number;
  title?: string;
  keyword?: string;
  searchType?: "all" | "content" | "author";
  isFeatured?: string;
  isTop?: string;
  sort?: "hot" | "new" | "featured";
  /** 1 = 只返回带投影文件的帖子；0 = 只返回不带投影文件的；不传 = 不限制 */
  hasProjection?: 0 | 1;
  pageNum?: number;
  pageSize?: number;
}

/** 下载完成的附件（`path` 可直接交给投影预览打开） */
export interface ShimuDownloadResult {
  path: string;
  name: string;
  size: number;
}

/** 详情页图库单张图片（Modrinth gallery / CurseForge screenshots） */
export interface ProjectGalleryImage {
  url: string;
  thumb?: string;
  title?: string;
  description?: string;
  featured?: boolean;
}

export interface ProjectHit {
  /** spigot 来源仅用于展示/翻译，不支持程序化安装 */
  provider: "modrinth" | "curseforge" | "spigot";
  id: string;
  slug: string;
  title: string;
  description: string;
  author: string;
  downloads: number;
  follows: number;
  icon_url: string;
  project_type: string;
  categories: string[];
  latest_version: string;
  game_versions: string[];
  updated: string;
  featured_image: string;
  /** 图库，仅详情接口返回；为空表示该项目没有图片 */
  gallery?: ProjectGalleryImage[];
}

export interface ProjectFile {
  url: string;
  filename: string;
  size: number;
  primary: boolean;
  hashes?: Record<string, string>;
}

export interface ProjectVersion {
  id: string;
  name: string;
  version_number: string;
  version_type?: string;
  date_published: string;
  game_versions: string[];
  loaders: string[];
  files: ProjectFile[];
  dependencies?: unknown[];
  download_url?: string;
  filename?: string;
  size?: number;
  release_type?: number;
}

export interface ProjectDependency {
  projectId: string;
  title: string;
  slug: string;
  dependencyType: "required" | "optional" | "incompatible" | "embedded" | string;
  versionId?: string;
}

export interface ContentItem {
  record: InstalledContent;
  exists: boolean;
}

export interface InstalledContent {
  filename: string;
  source: string;
  project_id: string | null;
  /** Modrinth/CurseForge 项目 slug，用于内容中心检索与中文名映射 */
  slug?: string | null;
  /** 后端按 WikiEntries 映射出的中文名（未命中为 null） */
  cn_name?: string | null;
  version_id: string | null;
  name: string | null;
  version: string | null;
  /** Mod 内部 id（fabric 的 sodium / forge 的 modId） */
  mod_id?: string | null;
  /** 作者列表 */
  authors?: string[] | null;
  /** Mod 描述 */
  description?: string | null;
  installed_at: number;
  size: number;
  icon: string | null;
  enabled: boolean;
}

/** 世界存档备份快照元信息 */
export interface WorldBackupInfo {
  filename: string;
  size: number;
  /** unix 秒 */
  modified: number;
}

/** NBT 编辑：世界列表条目 */
export interface WorldSummary {
  /** saves 下的目录名，手动指定时是绝对路径 */
  dir: string;
  levelName: string | null;
  gameType: number | null;
  difficulty: number | null;
  seed: number | null;
  size: number;
  /** unix 秒 */
  modified: number;
  /** 有 playerdata 的玩家数量 */
  playerCount?: number;
  /** 第一个玩家名（usercache 解析不到时是 UUID 短串） */
  playerName?: string | null;
}

/** NBT 编辑：level.dat 表单字段 */
export interface WorldForm {
  levelName: string | null;
  gameType: number | null;
  difficulty: number | null;
  difficultyLocked: number | null;
  hardcore: number | null;
  /** 十进制字符串，保留 NBT Long 的完整精度 */
  seed: string | null;
  time: string | null;
  dayTime: string | null;
  raining: number | null;
  thundering: number | null;
  spawnX: number | null;
  spawnY: number | null;
  spawnZ: number | null;
  allowCommands: number | null;
  /** 游戏规则：值统一是字符串（"true"/"false"/数字） */
  gameRules: Record<string, string> | null;
  border: { centerX: number | null; centerZ: number | null; size: number | null } | null;
  /** 边界用扁平的 Border* 字段存（1.16-）还是 WorldBorder 子复合节点（部分版本） */
  borderFlat: boolean | null;
}

/** NBT 编辑：已生成的区块（从 region 文件头部读出） */
export interface ChunkEntry {
  cx: number;
  cz: number;
  /** 占用的字节数（按扇区对齐） */
  size: number;
  /** unix 秒 */
  modified: number;
}

/** 区块地图：存档里已生成区块的范围 */
export interface MapBounds {
  empty: boolean;
  minCx?: number;
  minCz?: number;
  maxCx?: number;
  maxCz?: number;
  chunks?: number;
  /** 方块坐标：区块最密集的区域中心（存档跨度大时用它当初始视野） */
  denseX?: number;
  denseZ?: number;
}

/** 区块地图：一次渲染的结果（每像素一个调色板下标） */
export interface MapImageData {
  originX: number;
  originZ: number;
  /** 每个像素代表多少方块 */
  step: number;
  width: number;
  height: number;
  /** 真正画进图里的区块数 */
  rendered: number;
  /** 拍平的 RGB（每 3 个一组） */
  palette: number[];
  /** base64：width*height 个调色板下标 */
  data: string;
  /** 图例：视野里占比最高的几种方块 */
  legend: { name: string; color: string; pixels: number }[];
}

/** NBT 编辑：单文件备份条目 */
export interface NbtBackupInfo {
  /** 相对世界目录的路径：level.dat 或 playerdata/<uuid>.dat */
  file: string;
  name: string;
  size: number;
  /** unix 秒 */
  modified: number;
}

/** NBT 编辑：玩家条目 */
export interface PlayerSummary {
  uuid: string;
  name: string;
  pos: number[];
  dimension: string | null;
  health: number | null;
}

/** NBT 编辑：玩家可编辑数据 */
export interface PlayerData {
  pos: number[];
  dimension: string;
  health: number;
  foodLevel: number;
  xpLevel: number;
  xpP: number;
  gameType: number;
  abilities: { flying: number; mayfly: number } | null;
}

/** NBT 编辑：物品槽位 */
export interface ItemSlot {
  /** 在 NBT 列表里的下标（写回时的兜底定位依据） */
  index: number;
  /** 实际槽位号（0-35 背包、36-39 护甲、40 副手） */
  slot: number;
  id: string;
  count: number;
  name: string | null;
  lore: string[];
  unbreakable: number;
  enchantments: { id: string; level: number }[];
  modifiers: {
    name: string;
    attribute: string;
    amount: number;
    operation: number;
    slot: string | null;
  }[];
  /** 1.20.5+ 的 components 格式 */
  modern: boolean;
}

/** NBT 树节点（树形模式） */
export interface NbtNode {
  name: string;
  type: string;
  value?: unknown;
  children?: NbtNode[];
}

/**
 * 时光机时间轴的一条记忆点（本地 + 云端合并后的统一形态）。
 *
 * 本地优先：`localPath` 有值时前端直接显示（不用先下载）；只有云端条目才回源下载。
 */
export interface TimelineShot {
  /** 唯一 key：`local:<实例>:<文件>` 或 `cloud:<release>:<asset>` */
  id: string;
  instanceId: string;
  /** 截图文件名（本地文件 / 云端附件名） */
  file: string;
  /** 触发来源：interval / world_enter / death / advance… / manual */
  trigger: string | null;
  /** unix 秒 */
  createdAt: number;
  /** 是否已经传上云端 */
  uploaded: boolean;
  /** 本地文件绝对路径（已删/纯云端条目为 null） */
  localPath: string | null;
  /** 截图那一刻游戏日志的末尾几行（老截图没有） */
  logSnippet: string | null;
  releaseId: number;
  assetId: number;
}

/** 时间轴上的世界快照事件（本地备份 / 云端快照），对应"回到那一刻" */
export interface TimelineSnapshot {
  kind: "local" | "cloud";
  instanceId: string | null;
  instanceName: string | null;
  /** 世界目录名（云端为 worldId） */
  world: string;
  worldName: string | null;
  /** unix 秒 */
  createdAt: number;
  size: number;
  /** 本地备份文件名（kind=local） */
  file?: string;
  /** 云端（kind=cloud） */
  releaseId?: number;
  assetId?: number;
}

/** 单次游玩（启动游戏 → 退出）；「冒险日志」按它把截图分组成"这次游玩" */
export interface PlaySession {
  id: number;
  instanceId: string;
  /** unix 秒 */
  startedAt: number;
  /** unix 秒；进程被系统直接杀掉时可能为 null（表示"未记录结束"） */
  endedAt: number | null;
}

/** 云端存档快照（GitHub Release；大存档可能有多个分卷附件） */
export interface CloudSnapshot {
  releaseId: number;
  tag: string;
  /** ISO8601 */
  createdAt: string;
  assetId: number;
  /** 全部分卷附件 id（按分卷顺序） */
  assetIds: number[];
  assetSize: number;
  /** 分卷数（1 = 单文件） */
  partCount: number;
  worldId: string;
  worldName: string | null;
  /** 上传时所属的实例（旧快照可能为 null） */
  instanceId: string | null;
  instanceName: string | null;
  gameVersion: string | null;
  sha256: string | null;
  /** 时光机：这张截图由什么触发（interval / death / world_enter / custom:<id> …） */
  trigger?: string | null;
}

/** 局域网同步服务状态（lan_sync_* 命令返回值） */
export interface LanSyncState {
  /** 推荐给手机连接的地址 */
  ip: string;
  port: number;
  token: string;
  /** 二维码内容：qookix://lan/pair?host=…&port=…&token=… */
  qrUrl: string;
  /** 本机所有候选局域网 IPv4（多网卡时给用户切换） */
  ips: string[];
}

/** 从存档 level.dat 里读出的世界信息 */
export interface WorldInfo {
  /** 种子的十进制字符串（64 位，可能超出 JS 安全整数范围）；随机种子世界为 null */
  seed: string | null;
  /** `Data.Version.Name`，如 "26.2"；老存档可能没有 */
  version: string | null;
}

/** 实例导出预览：一个可勾选条目 */
export interface ExportItem {
  key: string;
  label: string;
  size: number;
  hint?: string | null;
}

/** 实例导出预览：一组勾选项 */
export interface ExportGroup {
  key: string;
  label: string;
  /** 必含（游戏本体），不可取消 */
  required: boolean;
  hint?: string | null;
  items: ExportItem[];
}

export interface ExportPreview {
  name: string;
  mcVersion?: string;
  mc_version: string;
  loader: string;
  loader_version?: string | null;
  groups: ExportGroup[];
}

/** 诊断报告的一节 */
export interface DiagnosticSection {
  key: string;
  title: string;
  lines: string[];
  /** 长文本（日志等） */
  block?: string | null;
}

export interface DiagnosticReport {
  generated_at: number;
  sections: DiagnosticSection[];
  /** 完整 Markdown 报告（复制 / 另存用） */
  markdown: string;
  /** 已脱敏替换处数 */
  redactions: number;
}

/** 已持久化的历史诊断报告 */
export interface DiagnosticReportEntry {
  filename: string;
  generated_at: number;
  size: number;
}

/** 未登记模组在 Modrinth 上的识别结果 */
export interface IdentifiedMod {
  filename: string;
  projectId: string;
  versionId: string;
  name: string;
  /** "hash" 精确匹配 | "name" 按文件名猜测（可能错配） */
  confidence: string;
}

/** 导出时实际提交的选择 */
export interface ExportSelection {
  name?: string | null;
  version?: string | null;
  mods: string[];
  includeDisabledMods?: boolean;
  resourcepacks: string[];
  shaders: string[];
  screenshots: string[];
  worlds: string[];
  folders: string[];
  optionsTxt?: boolean;
  serversDat?: boolean;
  /** 在线来源的模组直接打包文件（默认只记引用，导入时重新下载） */
  bundleOnlineFiles?: boolean;
  /** 打包在线文件时仅限 Modrinth 来源（规避 CurseForge 分发协议限制） */
  modrinthOnly?: boolean;
}

/** 游玩时长统计（后端聚合） */
export interface PlaytimeStats {
  totalSeconds: number;
  byInstance: {
    id: string;
    name: string;
    icon: string | null;
    seconds: number;
    lastPlayed: number | null;
  }[];
  /** 最近 30 天，day 为 (unix+8h)/86400 的天数索引 */
  byDay: { day: number; seconds: number }[];
}

export interface UpdateInfo {
  filename: string;
  projectId: string;
  currentVersion: string | null;
  latestVersion: string;
  latestVersionId: string;
  projectTitle: string | null;
  kind: string;
  provider: string;
}

export interface MissingDependency {
  modId: string;
  requirement: string;
  kind: string;
  requiredBy: string[];
  disabledFile: string | null;
}

export interface DependencyReport {
  instanceId: string;
  checkedMods: number;
  unreadable: number;
  missing: MissingDependency[];
  duplicates: { modId: string; files: string[] }[];
}

export interface ResolvedMissingMod {
  modId: string;
  provider: string;
  projectId: string;
  slug: string;
  title: string;
  icon: string;
  downloads: number;
  latestVersionId: string;
  exact: boolean;
}

export interface InstallProgressEvent {
  taskId: number;
  stage: string;
  message: string;
  done: number;
  total: number;
  instanceId?: string;
  instanceName?: string;
  source?: string;
  cancellable?: boolean;
  ok?: boolean;
}

export interface InstallPlan {
  instance_id: string;
  total_bytes: number;
  file_count: number;
}

export interface LaunchResult {
  taskId: number;
  pid: number;
}

export interface DownloadProgressEvent {
  taskId: number;
  phase: string;
  done: number;
  total: number;
  current: string;
  ok: boolean;
  bytesDone?: number;
  bytesTotal?: number;
  verifiedBytes: number;
  ts?: number;
  activeFiles?: ActiveFile[];
}

/** 正在下载的文件，由后端 `download.rs` 每 400ms 上报一次实时字节数 */
export interface ActiveFile {
  name: string;
  bytesDone: number;
  bytesTotal: number;
}

export interface NewsItem {
  title: string;
  description?: string;
  content?: string;
  author?: string;
  time: number;
  image?: string;
  image_alt?: string;
  url?: string;
  important?: boolean;
}

export interface LaunchLogEvent {
  instanceId: string;
  stream: "out" | "err";
  line: string;
}

export interface LaunchProgressEvent {
  taskId: number;
  instanceId: string;
  step: string;
  progress: number;
}

export interface LaunchStateEvent {
  taskId: number;
  instanceId: string;
  state: "running" | "exited";
  pid: number;
  code: number | null;
}

// 实例的多人游戏服务器条目（来自游戏内 servers.json / servers.dat）
export interface ServerEntry {
  name: string;
  address: string;
  icon: string | null; // 原始 base64（无 data: 前缀）
}

/** 实例文件管理器中的一条目录项 */
export interface FsEntry {
  name: string;
  /** 相对实例根目录的路径（用 / 分隔） */
  rel: string;
  size: number;
  modified: number;
  is_dir: boolean;
  /** 小写扩展名，目录为空字符串 */
  ext: string;
}

export interface DirectoryListing {
  rel: string;
  entries: FsEntry[];
}

export interface FileInfo {
  rel: string;
  size: number;
  modified: number;
}

export interface FileDocument extends FileInfo {
  content: string;
}

export interface CreatedEntry {
  rel: string;
  is_dir: boolean;
}

export interface RenamedEntry {
  rel: string;
  name: string;
}

// 经 Server List Ping 获取的实时状态
export interface ServerStatus {
  online: boolean;
  address: string;
  name: string | null;
  version: string | null;
  players_online: number | null;
  players_max: number | null;
  motd: string | null;
  favicon: string | null; // 完整 data:image/png;base64,...
  latency_ms: number | null;
  error: string | null;
}

// 本地托管的游戏服务器核心类型
export type ServerCore =
  | "vanilla"
  | "paper"
  | "spigot"
  | "purpur"
  | "forge"
  | "fabric";

// 陶瓦联机（Terracotta）
export interface TerracottaInfo {
  found: boolean;
  path: string | null;
  running: boolean;
  port: number | null;
  download_url: string;
  icon: string | null;
}

export interface TerracottaLaunch {
  port: number;
  ui_url: string;
  path: string;
}

// 陶瓦联机下载进度
export interface TerracottaDownloadProgress {
  downloaded: number;
  total: number;
  percent: number;
  extracting?: boolean;
  done?: boolean;
}

export type TerracottaRoomState =
  | "waiting"
  | "scanning"
  | "host-starting"
  | "host-ok"
  | "guest-connecting"
  | "guest-starting"
  | "guest-ok"
  | "exception";

// 用户在"多人游戏 → 服务器"中创建的本地服务器配置
export interface ServerConfig {
  id: string;
  name: string;
  core: ServerCore;
  mc_version: string;
  port: number;
  max_memory_mb: number;
  min_memory_mb: number;
  motd: string;
  eula: boolean;
  created: number;
  last_started: number | null;
  java_path: string | null;
  jvm_args: string | null;
  stop_command: string | null;
}

// 崩溃分析结果
// 主因字段（severity/title/reason/advice）取自置信度最高的一条，兼容旧展示逻辑；
// causes 里是全量命中原因，stacktrace/details 是报告结构化解析结果。
export type CrashSeverity =
  | "oom"
  | "jvm"
  | "lwjgl"
  | "java_ver"
  | "gl"
  | "mod"
  | "unknown";

export interface CrashCause {
  id: string;
  severity: CrashSeverity;
  title: string;
  reason: string;
  advice: string;
  /** 命中该原因的证据（崩溃报告原文片段） */
  evidence: string;
  /** 置信度 0-100 */
  confidence: number;
}

export interface CrashDetail {
  key: string;
  value: string;
}

export interface CrashDiagnosis {
  severity: CrashSeverity;
  title: string;
  reason: string;
  advice: string;
  excerpt: string;
  exit_code: number | null;
  crash_report: string | null;
  affected_mods: string[];
  /** 全部命中原因，按置信度降序 */
  causes: CrashCause[];
  /** 关键堆栈帧 */
  stacktrace: string[];
  /** 环境信息（Minecraft / Java / 内存 / 显卡 / 系统） */
  details: CrashDetail[];
  /** 主因置信度 0-100，0 表示未能定位 */
  confidence: number;
}

export interface CrashAICause {
  title: string;
  confidence: number;
  evidence: string;
  next_steps: string[];
}

export interface CrashAIResult {
  summary: string;
  likely_causes: CrashAICause[];
  need_more_info: string[];
  disclaimer: string;
}

/** 服务端 AI 深入分析结果；本地规则诊断仍以 CrashDiagnosis 为准 */
export interface CrashAIAnalysis {
  result: CrashAIResult;
  cached: boolean;
  model: string;
  hash: string;
}

// ---- QookiX 账号（可选）：好友 + 陶瓦联机房间目录 --------------
// 字段名沿用服务端 snake_case，避免中间层再做一次改名。

/** 对方心跳上报的「正在玩什么」，全部可选 */
export interface QookixActivity {
  instance?: string;
  mc_version?: string;
  loader?: string;
  server?: string;
}

/** 他人视角的用户：主键是 uid（自己才是 id） */
export interface QookixUserBrief {
  uid: number;
  username: string;
  nickname: string;
  online: boolean;
  /** 仅对方在线且开启了上报时才有 */
  activity?: QookixActivity;
}

export interface QookixUser {
  id: number;
  username: string;
  email?: string;
  nickname: string;
  /** approved | pending | rejected；非 approved 时别人看到的是 username */
  nickname_status?: string;
  nickname_review_reason?: string;
  role: string;
  status: string;
  created_at: number;
  email_verified: boolean;
  /** 绑定的 Minecraft 用户名，空串表示未绑定 */
  mc_name?: string;
  /** 32 位小写十六进制（无连字符） */
  mc_uuid?: string;
  /** 是否经过 Minecraft Services 验证；false 表示旧的手填资料 */
  mc_verified?: boolean;
  mc_bound_at?: number;
}

/** 批量接口返回的绑定信息（只含 uid + mc 字段） */
export interface QookixProfile {
  uid: number;
  mc_name: string;
  mc_uuid: string;
  mc_verified?: boolean;
}

export interface QookixShare {
  roomId: string;
  code: string;
  title: string;
  visibility: string;
  /** 是否附带环境清单 */
  hasPack?: boolean;
  modsCount?: number;
}

// ---- 联机环境清单（Phase 2a）----

/** QookiX 账号自带的皮肤托管 */
export interface QookixSkin {
  /** 文件名（相对 /skins/），空串表示没上传 */
  skin: string;
  cape: string;
  /** classic | slim */
  model: string;
  /** approved | pending | rejected：未过审时别人拿到的是空皮肤 */
  status?: string;
  reason?: string;
  updated_at: number;
}

// ---- 内网穿透（frp 面板）----

export interface FrpUser {
  id: number;
  username: string;
  email?: string;
  balance?: number;
}

export interface FrpTunnel {
  id: number;
  name: string;
  protocol: string;
  local_ip?: string;
  local_port: number;
  remote_port?: number;
  custom_domain?: string | null;
  status: string;
  node_name?: string;
  node_hostname?: string;
  node_port?: number;
  billing_type?: string;
}

/** 在线来源：有它就不必走 P2P，客人自己重下更快 */
export interface PackSource {
  /** modrinth | curseforge */
  provider: string;
  project_id: string;
  version_id: string;
}

/** 清单里的一条文件（以磁盘为准生成） */
export interface PackFile {
  /** mod | resourcepack | shaderpack */
  kind: string;
  /** 文件名（不含目录） */
  name: string;
  mod_id?: string;
  version?: string;
  size: number;
  /** 内容 SHA1：比对与传输校验都靠它 */
  sha1?: string;
  source?: PackSource | null;
  /** 资源包 / 光影默认不同步 */
  optional?: boolean;
}

export interface PackManifest {
  /** 格式版本 */
  v: number;
  mc_version: string;
  loader: string;
  loader_version: string;
  files: PackFile[];
  /** 需要 P2P 传输的字节数 */
  need_bytes?: number;
  /** 有在线来源的文件数 */
  online_count?: number;
}

export interface PackDiff {
  /** 房主有、我没有 */
  missing: PackFile[];
  /** 我有、房主没有 */
  extra: PackFile[];
  /** 两边都有但内容/版本不同 */
  version_mismatch: PackFile[];
  mc_version_mismatch: boolean;
  loader_mismatch: boolean;
}

export interface PackManifestOut {
  manifest: string;
  modsCount: number;
  mcVersion: string;
  loader: string;
  /** 需要 P2P 传输的字节数 */
  needBytes: number;
}

export interface PackCompareOut {
  hasPack: boolean;
  ok: boolean;
  diff?: PackDiff | null;
  remoteMcVersion: string;
  remoteLoader: string;
  remoteMods: number;
  localMods: number;
  /** 需要从房主拉取的字节数（只算本地缺的、且无在线来源的） */
  pullBytes: number;
  extraCount: number;
}

export interface QookixStatus {
  loggedIn: boolean;
  expiresAt: number;
  user: QookixUser | null;
  share?: QookixShare | null;
}

/** 登录密码正确但账号开了两步验证：继续用 challenge_token + 动态码换令牌 */
export interface QookixTwoFactorChallenge {
  two_factor_required: true;
  challenge_token: string;
  expires_in?: number;
  methods?: string[];
}

export interface QookixRegistration {
  email_verification_required: boolean;
  email_sent: boolean | null;
  message: string | null;
}

export interface QookixTwoFactorStatus {
  enabled: boolean;
  enabled_at?: number;
  last_used_at?: number;
  recovery_remaining?: number;
}

export interface QookixTwoFactorSetup {
  secret: string;
  otpauth_uri: string;
  issuer?: string;
  account?: string;
  digits?: number;
  period_sec?: number;
  recovery_codes: string[];
}

export interface QookixRegisterPolicy {
  invite_required: boolean;
  turnstile_enabled: boolean;
  turnstile_site_key?: string;
  terms_required?: boolean;
  terms_url?: string;
  terms_version?: string;
  ip_daily_limit?: number;
}

export interface QookixServiceStatus {
  online: boolean;
  state: string;
  server_time: number;
  worker?: { running: boolean };
  speed?: { ready?: boolean; chars_per_min?: number; mods_per_min?: number; window_sec?: number };
  today?: { chars?: number; mods?: number; calls?: number; body_ok?: number };
}

export interface QookixStats {
  friends: number;
  pending_requests: number;
  pending_invites: number;
  active_rooms: number;
}

export interface QookixMe {
  user: QookixUser;
  stats: QookixStats;
  online: boolean;
}

/** 服务端实际还可能返回 accepted（历史申请状态直出） */
export type QookixRelation =
  | "none"
  | "friend"
  | "requested"
  | "incoming"
  | "blocked"
  | "blocked_by"
  | "accepted"
  | "rejected"
  | "cancelled";

export interface QookixSearchHit {
  user: QookixUserBrief;
  relation: QookixRelation;
}

export interface QookixFriend {
  user: QookixUserBrief;
  created_at: number;
}

export interface QookixRequest {
  id: number;
  from?: QookixUserBrief;
  to?: QookixUserBrief;
  message: string;
  status: string;
  created_at: number;
}

export interface QookixBlock {
  user: QookixUserBrief;
  created_at: number;
}

export interface QookixRoomMember {
  name: string;
  role: string;
}

export type QookixRoomStatus = "active" | "expired" | "closed" | "removed";

export interface QookixRoom {
  /** 24 位十六进制字符串，不是整数 */
  id: string;
  /** 仅 status=active 且可见时返回 */
  code?: string;
  owner: QookixUserBrief;
  visibility: "friends" | "public";
  title: string;
  /** 公开房标题审核状态；pending/rejected 时不进公开大厅 */
  title_status?: string;
  game_version: string;
  modpack: string;
  status: QookixRoomStatus;
  /** 玩法类型（固定枚举），空串表示未标注 */
  play_type?: string;
  /** 环境清单里的 mod 数量（列表接口只给这个 + pack_hash） */
  mods_count?: number;
  /** 清单指纹，内容变了就变 */
  pack_hash?: string;
  /** 完整清单：只在详情接口出现 */
  pack?: PackManifest;
  member_count: number;
  report_count: number;
  created_at: number;
  updated_at: number;
  last_heartbeat: number;
  expires_at: number;
  /** 成员快照：只在详情接口下发（房主客户端上报后才能拿到） */
  members?: QookixRoomMember[];
}

export interface QookixInvite {
  id: number;
  room_id: string;
  from: QookixUserBrief;
  title: string;
  visibility: string;
  status: string;
  /** 邀请是 pending，但房间可能已经过期/关闭 */
  room_status: QookixRoomStatus;
  created_at: number;
  expires_at: number;
}

export interface QookixNotifications {
  server_time: number;
  friend_requests: QookixRequest[];
  room_invites: QookixInvite[];
}

/** 命令层错误（src-tauri/src/qookix/api.rs::ApiError） */
export interface QookixApiError {
  status: number;
  code: string;
  message: string;
}
