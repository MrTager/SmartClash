import Foundation

public enum RulesSupport: Sendable {
    case provider
    case restart
    case builtin
}

public struct KernelCapabilities: Sendable {
    public let tun: Bool
    public let rules: RulesSupport
    public let connections: Bool
    public let liveSwitch: Bool
    public let latencyAPI: Bool
    public init(tun: Bool, rules: RulesSupport, connections: Bool, liveSwitch: Bool, latencyAPI: Bool) {
        self.tun = tun; self.rules = rules; self.connections = connections
        self.liveSwitch = liveSwitch; self.latencyAPI = latencyAPI
    }
}

public enum KernelKind: String, Codable, CaseIterable, Identifiable, Sendable {
    case mihomo
    case singBox = "singbox"
    case xray

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .mihomo: return "Mihomo"
        case .singBox: return "sing-box"
        case .xray: return "Xray"
        }
    }

    public var summary: String {
        switch self {
        case .mihomo:
            return "Clash 系内核，协议覆盖最全，规则集可在运行时热更新。"
        case .singBox:
            return "通用代理内核，支持 Clash 控制接口；切换流量模式或节点会重启内核。"
        case .xray:
            return "轻量 Xray 内核，支持常见 VMess / VLESS / Trojan / Shadowsocks；不支持 TUN、实时连接视图与在线规则更新。"
        }
    }

    public var binaryName: String {
        switch self {
        case .mihomo: return "mihomo"
        case .singBox: return "sing-box"
        case .xray: return "xray"
        }
    }

    public var capabilities: KernelCapabilities {
        switch self {
        case .mihomo:
            return KernelCapabilities(tun: true, rules: .provider, connections: true, liveSwitch: true, latencyAPI: true)
        case .singBox:
            return KernelCapabilities(tun: true, rules: .restart, connections: true, liveSwitch: true, latencyAPI: true)
        case .xray:
            return KernelCapabilities(tun: false, rules: .builtin, connections: false, liveSwitch: false, latencyAPI: false)
        }
    }

    public func supports(_ node: ProxyNode) -> Bool {
        KernelConversion.supportError(kind: self, payload: node.payload) == nil
    }
}
