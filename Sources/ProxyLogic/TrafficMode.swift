import Foundation

public enum TrafficMode: String, Codable, CaseIterable, Identifiable {
    case rule, global, direct
    public var id: String { rawValue }
    public var title: String {
        switch self { case .rule: return "规则代理"; case .global: return "全局代理"; case .direct: return "直连" }
    }
    public var explanation: String {
        switch self {
        case .rule: return "按自有规则仓库分流：国内域名和 IP 直连，其余流量走当前代理线路。"
        case .global: return "进入内核的业务流量全部走当前代理线路，跳过分流与屏蔽规则。"
        case .direct: return "进入内核的业务流量全部直连，不使用代理节点；系统代理和 TUN 可继续开启。"
        }
    }
}
