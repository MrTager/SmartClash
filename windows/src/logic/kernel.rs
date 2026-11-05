use crate::logic::convert;
use crate::logic::model::ProxyNode;
use serde::{Deserialize, Serialize};

pub const XRAY_API_PORT: u16 = 10085;
pub const XRAY_SOCKS_PORT: u16 = 7893;

/// 可切换的代理内核。配置字段保持稳定，旧版 preferences.json 缺省为 Mihomo。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KernelKind {
    #[default]
    Mihomo,
    SingBox,
    Xray,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulesSupport {
    /// 通过内核控制接口按需更新规则集（Mihomo）。
    Provider,
    /// 规则集由内核自行下载/刷新，手动更新需要重启内核（sing-box）。
    Restart,
    /// 使用内核内置的 geo 数据，不支持在线更新规则（Xray）。
    Builtin,
}

#[derive(Clone, Copy, Debug)]
pub struct Capabilities {
    pub tun: bool,
    pub rules: RulesSupport,
    pub connections: bool,
    pub traffic: bool,
    /// 切换节点是否无需重启内核。
    pub live_switch: bool,
    /// 是否提供按节点测速的控制接口。
    pub latency_api: bool,
    /// 切换流量模式是否需要重启内核。
    pub mode_switch_restarts: bool,
}

impl KernelKind {
    pub const ALL: [KernelKind; 3] = [KernelKind::Mihomo, KernelKind::SingBox, KernelKind::Xray];

    pub fn title(self) -> &'static str {
        match self {
            KernelKind::Mihomo => "Mihomo",
            KernelKind::SingBox => "sing-box",
            KernelKind::Xray => "Xray",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            KernelKind::Mihomo => "Clash 系内核，协议覆盖最全，规则集可在运行时热更新。",
            KernelKind::SingBox => "通用代理内核，支持 Clash 控制接口；切换流量模式会重启内核。",
            KernelKind::Xray => "轻量 Xray 内核，支持常见 VMess / VLESS / Trojan / Shadowsocks；不支持 TUN、实时连接视图与在线规则更新。",
        }
    }

    pub fn binary_name(self) -> &'static str {
        match self {
            KernelKind::Mihomo => "mihomo.exe",
            KernelKind::SingBox => "sing-box.exe",
            KernelKind::Xray => "xray.exe",
        }
    }

    pub fn capabilities(self) -> Capabilities {
        match self {
            KernelKind::Mihomo => Capabilities {
                tun: true,
                rules: RulesSupport::Provider,
                connections: true,
                traffic: true,
                live_switch: true,
                latency_api: true,
                mode_switch_restarts: false,
            },
            KernelKind::SingBox => Capabilities {
                tun: true,
                rules: RulesSupport::Restart,
                connections: true,
                traffic: true,
                live_switch: true,
                latency_api: true,
                mode_switch_restarts: true,
            },
            KernelKind::Xray => Capabilities {
                tun: false,
                rules: RulesSupport::Builtin,
                connections: false,
                traffic: true,
                live_switch: false,
                latency_api: false,
                mode_switch_restarts: true,
            },
        }
    }

    pub fn supports_node(self, node: &ProxyNode) -> bool {
        convert::support_error(self, &node.payload).is_none()
    }
}

/// 解析旧配置里的内核名称，未知值回退到 Mihomo。
pub fn parse_kernel(value: &str) -> Option<KernelKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "mihomo" | "clash" => Some(KernelKind::Mihomo),
        "singbox" | "sing-box" => Some(KernelKind::SingBox),
        "xray" => Some(KernelKind::Xray),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_names_round_trip() {
        for kind in KernelKind::ALL {
            let text = serde_json::to_string(&kind).unwrap();
            let parsed: KernelKind = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed, kind);
        }
        assert_eq!(parse_kernel("sing-box"), Some(KernelKind::SingBox));
        assert_eq!(parse_kernel("unknown"), None);
    }
}
