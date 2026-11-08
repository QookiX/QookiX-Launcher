//! 下载镜像源
//!
//! 直连 Mojang / Fabric / Forge 官方 CDN 在部分地区经常超时或极慢。本模块按规则
//! 把官方地址改写到镜像站。镜像配置保存在 `settings.json`，**每次请求都重新读
//! 取设置**，所以切换镜像后立即生效，无需重启启动器。
//!
//! 规则面向 BMCLAPI / OpenBMCLAPI 的公开接口，任何兼容该接口的镜像站都可作为
//! 「自定义镜像」直接填写根地址使用：
//!   /mc/game/version_manifest_v2.json   版本清单
//!   /version/{version}/json             版本 JSON
//!   /version/{version}/client           客户端 jar
//!   /version/{version}/server           服务端 jar
//!   /assets/{hash[0..2]}/{hash}         资源文件
//!   /maven/{path}                       依赖库 / Forge / Fabric / NeoForge
//!   /fabric-meta/{path}                 Fabric Meta
//!
//! 镜像缺失文件时，下载器会自动回退到官方地址（见 `download.rs`）。

use crate::state::AppState;
use serde_json::{json, Value};

/// 官方源 id（不做任何改写）
pub const OFFICIAL: &str = "official";
/// 自定义镜像 id（使用 `mirror_custom` 里的根地址）
pub const CUSTOM: &str = "custom";

/// 官方版本清单地址
pub const OFFICIAL_MANIFEST: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

pub struct MirrorPreset {
    pub id: &'static str,
    pub label: &'static str,
    /// 镜像站根地址；官方源为空串
    pub base: &'static str,
    pub desc: &'static str,
}

/// 内置镜像预设（顺序即前端展示顺序）
pub const PRESETS: &[MirrorPreset] = &[
    MirrorPreset {
        id: OFFICIAL,
        label: "官方源",
        base: "",
        desc: "Mojang / Fabric / Forge 官方地址，海外网络推荐",
    },
    MirrorPreset {
        id: "bmclapi",
        label: "BMCLAPI",
        base: "https://bmclapi2.bangbang93.com",
        desc: "国内公益镜像，覆盖游戏本体、资源文件与依赖库",
    },
];

/// 由设置解析出实际生效的镜像根地址（官方源返回空串）。
/// 公开版本便于在已持有 settings 读锁的地方复用，避免重复加锁。
pub fn resolve_from(id: &str, custom: &str) -> String {
    if id == CUSTOM {
        return custom.trim().trim_end_matches('/').to_string();
    }
    PRESETS
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.base.to_string())
        .unwrap_or_default()
}

/// 当前生效的镜像根地址；空串表示官方源（不做任何改写）
pub fn base_url(state: &AppState) -> String {
    let s = state.settings();
    resolve_from(&s.mirror, &s.mirror_custom)
}

/// 当前是否使用官方源
#[allow(dead_code)]
pub fn is_official(state: &AppState) -> bool {
    base_url(state).is_empty()
}

/// 镜像预设列表（供前端渲染）
pub fn presets() -> Value {
    Value::Array(
        PRESETS
            .iter()
            .map(|p| json!({ "id": p.id, "label": p.label, "base": p.base, "desc": p.desc }))
            .collect(),
    )
}

struct Rule {
    /// 官方地址前缀
    prefix: &'static str,
    /// 镜像侧对应的路径前缀（以 `/` 开头）
    target: &'static str,
}

/// 前缀越长越靠前，避免被更短的通用前缀抢先匹配
const RULES: &[Rule] = &[
    Rule {
        prefix: OFFICIAL_MANIFEST,
        target: "/mc/game/version_manifest_v2.json",
    },
    Rule {
        prefix: "https://files.minecraftforge.net/maven/",
        target: "/maven/",
    },
    Rule {
        prefix: "https://maven.neoforged.net/releases/",
        target: "/maven/",
    },
    Rule {
        prefix: "https://maven.neoforged.net/",
        target: "/maven/",
    },
    Rule {
        prefix: "https://maven.minecraftforge.net/",
        target: "/maven/",
    },
    Rule {
        prefix: "https://maven.quiltmc.org/repository/release/",
        target: "/maven/",
    },
    Rule {
        prefix: "https://maven.fabricmc.net/",
        target: "/maven/",
    },
    Rule {
        prefix: "https://libraries.minecraft.net/",
        target: "/maven/",
    },
    Rule {
        prefix: "https://repo1.maven.org/maven2/",
        target: "/maven/",
    },
    Rule {
        prefix: "https://resources.download.minecraft.net/",
        target: "/assets/",
    },
    Rule {
        prefix: "https://meta.fabricmc.net/",
        target: "/fabric-meta/",
    },
];

/// 把单个官方地址改写到镜像；无法识别或官方源时原样返回。
/// 无状态，可在下载线程中安全复用。
pub fn map(base: &str, url: &str) -> String {
    if base.is_empty() || url.is_empty() {
        return url.to_string();
    }
    for r in RULES {
        if let Some(rest) = url.strip_prefix(r.prefix) {
            return format!("{}{}{}", base, r.target, rest);
        }
    }
    url.to_string()
}

/// 按当前设置改写地址
pub fn rewrite(state: &AppState, url: &str) -> String {
    map(&base_url(state), url)
}

/// 版本清单地址
pub fn manifest_url(state: &AppState) -> String {
    let base = base_url(state);
    if base.is_empty() {
        OFFICIAL_MANIFEST.to_string()
    } else {
        format!("{base}/mc/game/version_manifest_v2.json")
    }
}

/// 版本 JSON：镜像源用 `/version/{id}/json`，官方源沿用版本清单里的地址
pub fn version_json_url(state: &AppState, official_url: &str, version_id: &str) -> String {
    let base = base_url(state);
    if base.is_empty() {
        official_url.to_string()
    } else {
        format!("{base}/version/{version_id}/json")
    }
}

/// 客户端 jar：镜像源用 `/version/{id}/client`，官方源沿用版本 JSON 里的地址
pub fn client_jar_url(state: &AppState, official_url: &str, version_id: &str) -> String {
    let base = base_url(state);
    if base.is_empty() {
        official_url.to_string()
    } else {
        format!("{base}/version/{version_id}/client")
    }
}

/// 服务端 jar：镜像源用 `/version/{id}/server`，官方源沿用版本 JSON 里的地址
pub fn server_jar_url(state: &AppState, official_url: &str, version_id: &str) -> String {
    let base = base_url(state);
    if base.is_empty() {
        official_url.to_string()
    } else {
        format!("{base}/version/{version_id}/server")
    }
}

/// 资源文件（assets object）
pub fn asset_url(state: &AppState, hash: &str) -> String {
    let head = hash.get(0..2).unwrap_or(hash);
    let base = base_url(state);
    if base.is_empty() {
        format!("https://resources.download.minecraft.net/{head}/{hash}")
    } else {
        format!("{base}/assets/{head}/{hash}")
    }
}

// ---------------------------------------------------------------------------
// 内容 API 镜像（发现页 / 内容详情 / 版本列表）
// ---------------------------------------------------------------------------
//
// 与上面的「下载镜像」是两套东西：下载镜像改写的是游戏本体/依赖库的文件地址，
// 这里改写的是 Modrinth / CurseForge 的 **查询接口**（搜索、项目详情、版本列表、
// 依赖解析）。国内直连 `api.modrinth.com` 经常几百毫秒起步甚至超时，发现页
// 首屏就会一直转圈；换成镜像后这些请求改走国内节点。
//
// 镜像约定：根地址下同时提供 `/modrinth/v2` 与 `/curseforge/v1`
// （与 MCIM 一类镜像保持一致）。也允许直接填平台专属地址（以 `/v2`、`/v1` 结尾），
// 此时只影响对应平台。

/// 官方 Modrinth API 根地址
pub const MODRINTH_API: &str = "https://api.modrinth.com/v2";
/// 官方 CurseForge API 根地址
pub const CURSEFORGE_API: &str = "https://api.curseforge.com/v1";

/// 内容 API 走官方源（默认）
pub const CONTENT_OFFICIAL: &str = "official";
/// 内容 API 走自定义镜像根地址
pub const CONTENT_CUSTOM: &str = "custom";

pub struct ContentPreset {
    pub id: &'static str,
    pub label: &'static str,
    /// 镜像根地址；官方源为空串
    pub base: &'static str,
    pub desc: &'static str,
}

/// 内置内容 API 镜像预设（顺序即前端展示顺序）
pub const CONTENT_PRESETS: &[ContentPreset] = &[
    ContentPreset {
        id: CONTENT_OFFICIAL,
        label: "官方源",
        base: "",
        desc: "Modrinth / CurseForge 官方接口，海外网络推荐",
    },
    ContentPreset {
        id: "mcim",
        label: "MCIM 镜像",
        base: "https://mod.mcimirror.top",
        desc: "国内第三方镜像，加速发现页搜索与内容详情",
    },
];

/// 由设置解析出内容镜像根地址（官方源返回空串）。
pub fn content_resolve_from(id: &str, custom: &str) -> String {
    if id == CONTENT_CUSTOM {
        return custom.trim().trim_end_matches('/').to_string();
    }
    CONTENT_PRESETS
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.base.to_string())
        .unwrap_or_default()
}

/// 由镜像根地址推导出某个平台的 API 根地址。
///
/// `platform` 只认 `"modrinth"` / `"curseforge"`（其余按 modrinth 处理）。
/// 根地址为空 = 官方源；已带 `/v2`（或 `/v1`）时视为用户直接填了平台专属地址。
pub fn content_api_from(root: &str, platform: &str) -> String {
    let root = root.trim().trim_end_matches('/');
    let (official, suffix, path) = if platform == "curseforge" {
        (CURSEFORGE_API, "/v1", "/curseforge/v1")
    } else {
        (MODRINTH_API, "/v2", "/modrinth/v2")
    };
    if root.is_empty() {
        return official.to_string();
    }
    if root.ends_with(suffix) {
        root.to_string()
    } else {
        format!("{root}{path}")
    }
}

/// 当前设置下某个平台的 API 根地址（每次请求都读设置，切换后立即生效）。
pub fn content_api(state: &AppState, platform: &str) -> String {
    let s = state.settings();
    let root = content_resolve_from(&s.content_mirror, &s.content_mirror_custom);
    drop(s);
    content_api_from(&root, platform)
}

/// 内容镜像预设列表（供前端渲染）
pub fn content_presets() -> Value {
    Value::Array(
        CONTENT_PRESETS
            .iter()
            .map(|p| json!({ "id": p.id, "label": p.label, "base": p.base, "desc": p.desc }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_api_follows_mirror_root() {
        // 官方源：两端各自回官方地址
        assert_eq!(content_api_from("", "modrinth"), MODRINTH_API);
        assert_eq!(content_api_from("", "curseforge"), CURSEFORGE_API);
        // 镜像根：拼平台前缀
        assert_eq!(
            content_api_from("https://mod.mcimirror.top/", "modrinth"),
            "https://mod.mcimirror.top/modrinth/v2"
        );
        assert_eq!(
            content_api_from("https://mod.mcimirror.top", "curseforge"),
            "https://mod.mcimirror.top/curseforge/v1"
        );
        // 直接填平台专属地址：不再重复拼前缀
        assert_eq!(
            content_api_from("https://proxy.example.com/v2", "modrinth"),
            "https://proxy.example.com/v2"
        );
    }

    #[test]
    fn content_resolve_ignores_unknown_id() {
        assert_eq!(content_resolve_from("official", "https://x"), "");
        assert_eq!(content_resolve_from("nope", "https://x"), "");
        assert_eq!(
            content_resolve_from(CONTENT_CUSTOM, " https://x/ "),
            "https://x"
        );
    }
}
