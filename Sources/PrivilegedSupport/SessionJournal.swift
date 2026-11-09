import Foundation
import LifecycleSupport

public enum ProxyFieldValue: Codable, Equatable, Sendable {
    case text(String), number(Int)
}
public struct ProxyServiceChange: Codable, Equatable, Sendable {
    public var service: String
    public var original: [String: ProxyFieldValue]
    public var applied: [String: ProxyFieldValue]
    public init(service: String, original: [String: ProxyFieldValue], applied: [String: ProxyFieldValue]) {
        self.service = service; self.original = original; self.applied = applied
    }
}
public struct ProxyRestorePlan: Equatable {
    public var writes: [String: ProxyFieldValue]
    public var removals: Set<String>
    public var conflicts: Set<String>
    public var complete: Bool { conflicts.isEmpty }
}
public enum ProxyReconciliation {
    // 字段原值缺失也是有效状态。外部修改不能被整份快照覆盖。
    public static func plan(change: ProxyServiceChange, current: [String: ProxyFieldValue]) -> ProxyRestorePlan {
        var result = ProxyRestorePlan(writes: [:], removals: [], conflicts: [])
        for key in change.applied.keys {
            if current[key] == change.original[key] { continue }
            guard current[key] == change.applied[key] else { result.conflicts.insert(key); continue }
            if let value = change.original[key] { result.writes[key] = value }
            else { result.removals.insert(key) }
        }
        return result
    }
}
public struct HelperSessionJournal: Codable, Sendable {
    public var nonce: String
    public var owner: ProcessIdentity
    public var phase: String
    public var proxies: [ProxyServiceChange]
    public var tun: Bool
    public var reports: [String]
    public init(nonce: String, owner: ProcessIdentity) {
        self.nonce = nonce; self.owner = owner; phase = "ready"; proxies = []; tun = false; reports = []
    }
    public var clean: Bool { proxies.isEmpty && !tun && phase == "ended" }
}
public protocol HelperSessionBackend: AnyObject {
    func snapshot(port: Int) throws -> [ProxyServiceChange]
    func apply(_ changes: [ProxyServiceChange]) throws
    func restore(_ changes: [ProxyServiceChange]) throws -> (remaining: [ProxyServiceChange], reports: [String])
    func startTun(_ data: Data, kernel: String, port: Int) throws
    func stopTun() throws
}

// 所有入口由 helper 串行队列调用；测试注入内存后端，不接触系统网络。
public final class HelperSessionController {
    public private(set) var journal: HelperSessionJournal?
    private let backend: HelperSessionBackend
    private let save: (HelperSessionJournal) throws -> Void
    private let observe: (Int32) -> ProcessObservation
    public init(journal: HelperSessionJournal?, backend: HelperSessionBackend,
                save: @escaping (HelperSessionJournal) throws -> Void,
                observe: @escaping (Int32) -> ProcessObservation = ProcessIdentity.query) {
        self.journal = journal; self.backend = backend; self.save = save; self.observe = observe
    }
    private func persist() throws { if let journal { try save(journal) } }
    public func reconcile() throws {
        guard journal != nil, journal?.clean != true else { return }
        try cleanup()
    }
    public func begin(nonce: String, owner: ProcessIdentity) throws {
        guard UUID(uuidString: nonce) != nil, owner.uid > 0,
              Ownership.compare(owner, observe(owner.pid)) == .same else { throw LifecycleError.refused("会话 owner 身份未知") }
        if let old = journal, !old.clean {
            guard old.nonce == nonce, old.owner == owner, old.phase == "ready" else { throw LifecycleError.refused("旧网络会话尚未恢复") }
            return
        }
        journal = HelperSessionJournal(nonce: nonce, owner: owner); try persist()
    }
    public func authorize(nonce: String, owner: ProcessIdentity, cleanup: Bool = false) throws {
        guard let journal, journal.nonce == nonce, journal.owner == owner,
              Ownership.compare(owner, observe(owner.pid)) == .same,
              cleanup || journal.phase == "ready" else { throw LifecycleError.refused("网络会话认证失败或正在清理") }
    }
    public func checkOwner() throws {
        guard let journal, !journal.clean else { return }
        if Ownership.compare(journal.owner, observe(journal.owner.pid)) == .gone { try cleanup() }
    }
    public func setProxy(port: Int, enabled: Bool) throws {
        guard journal != nil else { throw LifecycleError.refused("缺少网络会话") }
        if enabled {
            guard journal!.proxies.isEmpty else { throw LifecycleError.refused("已有代理恢复记录") }
            journal!.proxies = try backend.snapshot(port: port)
            journal!.phase = "applying-proxy"; try persist()
            do { try backend.apply(journal!.proxies); journal!.phase = "ready"; try persist() }
            catch { try? cleanup(); throw error }
        } else { try restoreProxy(); journal!.phase = "ready"; try persist() }
    }
    private func restoreProxy() throws {
        guard let changes = journal?.proxies, !changes.isEmpty else { return }
        let result = try backend.restore(changes)
        journal!.proxies = result.remaining; journal!.reports = result.reports; try persist()
        guard result.remaining.isEmpty else { throw LifecycleError.refused(result.reports.joined(separator: "；")) }
    }
    public func startTun(_ data: Data, kernel: String, port: Int) throws {
        guard journal != nil, journal!.tun == false else { throw LifecycleError.refused("TUN 恢复记录尚未清理") }
        journal!.tun = true; journal!.phase = "starting-tun"; try persist()
        do { try backend.startTun(data, kernel: kernel, port: port); journal!.phase = "ready"; try persist() }
        catch { try? cleanup(); throw error }
    }
    public func stopTun() throws {
        guard journal?.tun == true else { return }
        try backend.stopTun(); journal!.tun = false; try persist()
    }
    public func cleanup() throws {
        guard journal != nil else { return }
        journal!.phase = "cleaning"; try persist()
        var failures: [String] = []
        do { try restoreProxy() } catch { failures.append(error.localizedDescription) }
        do { try stopTun() } catch { failures.append(error.localizedDescription) }
        journal!.reports = failures; journal!.phase = failures.isEmpty ? "ended" : "recovery"
        try persist()
        if !failures.isEmpty { throw LifecycleError.refused(failures.joined(separator: "；")) }
    }
}

public struct HelperSessionStatus: Codable, Sendable {
    public var phase: String
    public var proxyPending: Bool
    public var tunPending: Bool
    public var reports: [String]
    public init(_ journal: HelperSessionJournal?) {
        phase = journal?.phase ?? "absent"; proxyPending = !(journal?.proxies.isEmpty ?? true)
        tunPending = journal?.tun ?? false; reports = journal?.reports ?? []
    }
}
