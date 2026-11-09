import Foundation
import SystemConfiguration
import PrivilegedSupport
import LifecycleSupport

// 同一个非阻塞 SCPreferences 锁内读、比较、提交，避免 networksetup 多命令中间态。
final class ProxyPreferences {
    let preferences: SCPreferences
    let services: [SCNetworkService]
    init() throws {
        guard let preferences = SCPreferencesCreate(nil, "SmartClashHelper" as CFString, nil),
              SCPreferencesLock(preferences, false) else { throw LifecycleError.refused("系统网络偏好正被其他程序修改") }
        self.preferences = preferences
        SCPreferencesSynchronize(preferences)
        services = (SCNetworkServiceCopyAll(preferences) as? [SCNetworkService]) ?? []
    }
    deinit { SCPreferencesUnlock(preferences) }
    private func proxy(_ id: String) throws -> SCNetworkProtocol {
        guard let service = services.first(where: { SCNetworkServiceGetServiceID($0) as String == id }),
              let proxy = SCNetworkServiceCopyProtocol(service, kSCNetworkProtocolTypeProxies) else {
            throw LifecycleError.refused("恢复记录中的网络服务已不存在：\(id)")
        }
        return proxy
    }
    private func dictionary(_ proxy: SCNetworkProtocol) -> [String: Any] {
        (SCNetworkProtocolGetConfiguration(proxy) as? [String: Any]) ?? [:]
    }
    private func values(_ dictionary: [String: Any]) -> [String: ProxyFieldValue] {
        var values: [String: ProxyFieldValue] = [:]
        for (key, value) in dictionary {
            if let text = value as? String { values[key] = .text(text) }
            else if let number = value as? NSNumber { values[key] = .number(number.intValue) }
        }
        return values
    }
    func snapshot(port: Int) throws -> [ProxyServiceChange] {
        let applied: [String: ProxyFieldValue] = ["HTTPEnable": .number(1), "HTTPProxy": .text("127.0.0.1"), "HTTPPort": .number(port),
            "HTTPSEnable": .number(1), "HTTPSProxy": .text("127.0.0.1"), "HTTPSPort": .number(port),
            "SOCKSEnable": .number(1), "SOCKSProxy": .text("127.0.0.1"), "SOCKSPort": .number(port)]
        var changes: [ProxyServiceChange] = []
        for service in services where SCNetworkServiceGetEnabled(service) {
            guard let proxy = SCNetworkServiceCopyProtocol(service, kSCNetworkProtocolTypeProxies) else { continue }
            let raw = dictionary(proxy), current = values(raw)
            for key in ["ProxyAutoConfigEnable", "ProxyAutoDiscoveryEnable", "HTTPProxyAuthenticated", "HTTPSProxyAuthenticated", "SOCKSProxyAuthenticated"] {
                if case .number(let value) = current[key], value != 0 { throw LifecycleError.refused("网络服务启用自动或认证代理，拒绝覆盖") }
            }
            guard applied.keys.allSatisfy({ raw[$0] == nil || current[$0] != nil }) else { throw LifecycleError.refused("代理原值类型未知，拒绝覆盖") }
            changes.append(.init(service: SCNetworkServiceGetServiceID(service) as String,
                original: current.filter { applied[$0.key] != nil }, applied: applied))
        }
        guard !changes.isEmpty, changes.count <= 512 else { throw LifecycleError.refused("没有可安全修改的网络服务") }
        return changes
    }
    private func set(_ values: [String: ProxyFieldValue], removals: Set<String> = [], proxy: SCNetworkProtocol) throws {
        var raw = dictionary(proxy)
        for key in removals { raw[key] = nil }
        for (key, value) in values {
            switch value { case .text(let text): raw[key] = text; case .number(let number): raw[key] = number }
        }
        guard SCNetworkProtocolSetConfiguration(proxy, raw as CFDictionary) else { throw LifecycleError.refused("网络偏好设置更新失败") }
    }
    private func commit() throws {
        guard SCPreferencesCommitChanges(preferences), SCPreferencesApplyChanges(preferences) else { throw LifecycleError.refused("网络偏好提交或应用未完成，保留恢复日志") }
    }
    func apply(_ changes: [ProxyServiceChange]) throws {
        for change in changes { try set(change.applied, proxy: proxy(change.service)) }
        try commit()
    }
    func restore(_ changes: [ProxyServiceChange]) throws -> (remaining: [ProxyServiceChange], reports: [String]) {
        var remaining: [ProxyServiceChange] = [], reports: [String] = []
        for change in changes {
            do {
                let proxy = try proxy(change.service), current = values(dictionary(proxy))
                let plan = ProxyReconciliation.plan(change: change, current: current)
                try set(plan.writes, removals: plan.removals, proxy: proxy)
                if !plan.complete { remaining.append(change); reports.append("\(change.service)：外部代理字段冲突 \(plan.conflicts.sorted().joined(separator: ","))") }
            } catch { remaining.append(change); reports.append(error.localizedDescription) }
        }
        try commit()
        return (remaining, reports)
    }
}
