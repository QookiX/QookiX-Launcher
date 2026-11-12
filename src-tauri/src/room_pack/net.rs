//! P2P 连接信息与 EasyTier 进程管理。
//!
//! 连接信息只有一行，走服务端：
//! `qkx1|<network>|<secret>|<peer-addr>|<vip>|<http-port>|<token>`
//!
//! 本机实测（EasyTier 2.5.0，`--no-tun`）定下的三条硬约束：
//!
//! 1. **不能传 `--p2p-only`**。它的官方说明是「禁用 P2P 通信，只通过 --peers 中转」，
//!    和直觉相反；带上它节点之间不会直连。
//! 2. **peers 地址不能用 127.0.0.1**。`--no-tun` 下回环连接会直接 `WSAEADDRNOTAVAIL(10049)`，
//!    必须用真实网卡 IP（实测 `tcp://192.168.1.114:11013` 能连，`tcp://127.0.0.1:11013` 不能）。
//! 3. **两台节点的 `-r` 必须错开**，默认都是 15888 会撞。
//!
//! 安全边界（docs/room-pack-sync-design.md §9）：公开大厅的元数据人人可读，
//! 所以 `secret` 不能当 ACL。真正的边界是 HTTP 服务只认清单里登记过的文件。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// 一套连接信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackNet {
    pub network: String,
    pub secret: String,
    /// 房主的监听地址 `ip:port`，客人用它接入同一个 EasyTier 网络
    pub peer_addr: String,
    /// 房主在虚拟网里的 IP，port-forward 的目标
    pub vip: String,
    /// 房主 HTTP 服务端口（在 vip 上）
    pub http_port: u16,
    pub token: String,
}

impl PackNet {
    pub fn encode(&self) -> String {
        format!(
            "qkx1|{}|{}|{}|{}|{}|{}",
            self.network, self.secret, self.peer_addr, self.vip, self.http_port, self.token
        )
    }

    pub fn parse(s: &str) -> Option<Self> {
        let mut it = s.trim().split('|');
        if it.next()? != "qkx1" {
            return None;
        }
        let network = it.next()?.to_string();
        let secret = it.next()?.to_string();
        let peer_addr = it.next()?.to_string();
        let vip = it.next()?.to_string();
        let http_port = it.next()?.parse().ok()?;
        let token = it.next()?.to_string();
        if network.is_empty() || peer_addr.is_empty() || vip.is_empty() || token.is_empty() {
            return None;
        }
        Some(PackNet {
            network,
            secret,
            peer_addr,
            vip,
            http_port,
            token,
        })
    }
}

/// 本机对外可达的 IPv4：连一个外部地址（UDP connect 不发包）读出来即可，
/// 比枚举网卡可靠——能自动跳过 127.0.0.1 / 虚拟网卡 / 未连接适配器。
pub fn lan_ipv4() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("223.5.5.5:80").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        return None;
    }
    Some(ip.to_string())
}

/// 找陶瓦释放出来的 EasyTier 可执行文件。
///
/// 陶瓦每次运行会解压到 `%LOCALAPPDATA%\Temp\terracotta\<时间戳>\embedded-easytier\`，
/// 目录名是时间戳，所以取**最新**的那个。找不到就返回 None（上层降级到手动导入）。
pub fn locate_easytier() -> Option<(PathBuf, PathBuf)> {
    let base = std::env::var("LOCALAPPDATA").ok()?;
    let root = Path::new(&base).join("Temp").join("terracotta");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    // 目录名是时间戳，字符串倒序即「最新优先」
    dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    for dir in dirs {
        let emb = dir.join("embedded-easytier");
        let core = emb.join("easytier-core.exe");
        let cli = emb.join("easytier-cli.exe");
        if core.is_file() && cli.is_file() {
            return Some((core, cli));
        }
    }
    None
}

/// 起一个 `--no-tun` 节点：不建虚拟网卡、不需要管理员权限、不改系统路由。
///
/// - 房主传 `listen`（`tcp://<lan-ip>:<port>`）对外提供接入点
/// - 客人传 `peer`（`tcp://<房主 ip:port>`）连过去
pub fn spawn_node(
    core: &Path,
    net: &PackNet,
    ipv4: &str,
    rpc_port: u16,
    listen: Option<&str>,
    peer: Option<&str>,
    whitelist_ports: &[u16],
) -> Result<Child, String> {
    let mut cmd = Command::new(core);
    cmd.arg("--network-name")
        .arg(&net.network)
        .arg("--network-secret")
        .arg(&net.secret)
        // 注意：这里**不能**加 --p2p-only（它的含义是禁用 P2P）
        .arg("--no-tun")
        .arg("-i")
        .arg(ipv4)
        .arg("-r")
        .arg(format!("127.0.0.1:{rpc_port}"));
    if let Some(l) = listen {
        cmd.arg("-l").arg(l);
    }
    if let Some(p) = peer {
        // 客人不需要对外监听：不加 --no-listener 时它会默认占 UDP 11010，
        // 同机双开（或房主也在这台机器）就会撞端口，第二个节点直接起不来。
        cmd.arg("--no-listener");
        cmd.arg("-p").arg(p);
    }
    for port in whitelist_ports {
        cmd.arg(format!("--tcp-whitelist={port}"));
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("启动 EasyTier 节点失败: {e}"))
}

/// 把对端虚拟 IP 的端口映射到本地回环，之后直接访问 127.0.0.1 即可
pub fn port_forward(
    cli: &Path,
    rpc_port: u16,
    local: u16,
    remote_ip: &str,
    remote_port: u16,
) -> Result<(), String> {
    let out = Command::new(cli)
        .args([
            "-p",
            &format!("127.0.0.1:{rpc_port}"),
            "port-forward",
            "add",
            "tcp",
            &format!("127.0.0.1:{local}"),
            &format!("{remote_ip}:{remote_port}"),
        ])
        .output()
        .map_err(|e| format!("调用 easytier-cli 失败: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "port-forward 失败: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

/// 进程退出兜底：先 try_wait，没死才 kill
pub fn kill_child(child: &mut Child) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PackNet {
        PackNet {
            network: "qkx-abc123".into(),
            secret: "JRB2-0RRL".into(),
            peer_addr: "192.168.1.114:11013".into(),
            vip: "10.144.171.1".into(),
            http_port: 18099,
            token: "deadbeefdeadbeef".into(),
        }
    }

    #[test]
    fn pack_net_roundtrip() {
        let n = sample();
        let s = n.encode();
        assert!(s.starts_with("qkx1|"));
        assert_eq!(PackNet::parse(&s), Some(n));
    }

    #[test]
    fn pack_net_rejects_garbage() {
        assert_eq!(PackNet::parse(""), None);
        assert_eq!(PackNet::parse("qkx2|a|b|c|d|1|e"), None);
        // http_port 不是数字
        assert_eq!(PackNet::parse("qkx1|a|b|c|d|notaport|e"), None);
        // token 为空
        assert_eq!(PackNet::parse("qkx1|a|b|c|d|8080|"), None);
    }

    /// 不硬断言：把「本机能不能找到 EasyTier / 局域网 IP 是多少」打出来，
    /// 便于在真机上确认那条脆弱的临时目录约定还成立。
    #[test]
    fn locate_env_probe() {
        println!("[probe] lan_ipv4={:?}", lan_ipv4());
        match locate_easytier() {
            Some((core, cli)) => println!(
                "[probe] found core={} cli={}",
                core.display(),
                cli.display()
            ),
            None => println!("[probe] EasyTier not found (陶瓦未运行过?)"),
        }
    }
}
