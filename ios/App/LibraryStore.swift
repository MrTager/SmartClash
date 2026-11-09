import Foundation
import Combine

@MainActor
final class LibraryStore: ObservableObject {
    @Published private(set) var library = MobileLibrary()
    @Published private(set) var nodes: [ProxyNode] = []
    @Published private(set) var busy = false
    @Published private(set) var writable = false
    @Published var error: String?
    @Published var notice: String?
    private let storage = SecureLibrary()
    var selected: ProxyNode? { nodes.first { $0.id == library.selectedNodeID } }
    init() { reload() }
    func reload() {
        do {
            let loaded = try storage.load()
            let parsed = try MobileConfiguration.parsed(loaded)
            library = loaded; nodes = parsed; writable = true
        } catch { writable = false; self.error = error.localizedDescription }
    }
    private func commit(_ next: MobileLibrary) throws {
        guard writable else { throw MobileFailure.message("资料库未成功解锁，禁止覆盖；请解锁后重试读取。") }
        let parsed = try MobileConfiguration.parsed(next)
        try storage.save(next)
        library = next; nodes = parsed
    }
    func choose(_ node: ProxyNode) { mutate { $0.selectedNodeID = node.id } }
    func mode(_ mode: TrafficMode) { mutate { $0.trafficMode = mode } }
    func preference(_ mode: LinkPreference) { mutate { $0.linkPreference = mode } }
    private func mutate(_ change: (inout MobileLibrary) -> Void) {
        guard !busy else { return }
        do { var next = library; change(&next); try commit(next) }
        catch { self.error = error.localizedDescription }
    }
    func remove(_ id: UUID) {
        mutate { $0.subscriptions.removeAll { $0.id == id } }
    }
    private func upsert(source: Source, content: String) throws {
        let imported = try SubscriptionParser.parse(content, source: source)
        guard !imported.isEmpty else { throw MobileFailure.message("订阅没有可用节点。") }
        var next = library
        let item = SubscriptionRecord(source: source, content: content, updatedAt: Date())
        if let index = next.subscriptions.firstIndex(where: { $0.id == source.id }) { next.subscriptions[index] = item }
        else { next.subscriptions.append(item) }
        let merged = try MobileConfiguration.parsed(next)
        if !merged.contains(where: { $0.id == next.selectedNodeID }) { next.selectedNodeID = merged.first?.id }
        try commit(next)
    }
    func add(name: String, address: String) async -> Bool {
        guard !busy && writable else { return false }
        let title = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty else { error = "请输入订阅名称。"; return false }
        busy = true; defer { busy = false }
        do {
            let url = try SubscriptionDownload.validateURL(address)
            guard !library.subscriptions.contains(where: { $0.source.url == url.absoluteString }) else {
                throw MobileFailure.message("该订阅已经存在。")
            }
            let content = try await SubscriptionDownload.fetch(url.absoluteString)
            try upsert(source: Source(name: String(title.prefix(80)), url: url.absoluteString), content: content)
            notice = "订阅已导入，凭据仅加密保存在本设备。"
            return true
        } catch is CancellationError { return false }
        catch { self.error = error.localizedDescription; return false }
    }
    func refresh(_ item: SubscriptionRecord) async {
        guard !busy && writable && !item.source.isLocal else { return }
        busy = true; defer { busy = false }
        do {
            let content = try await SubscriptionDownload.fetch(item.source.url)
            try upsert(source: item.source, content: content)
            notice = "订阅已更新。"
        } catch is CancellationError { }
        catch { self.error = error.localizedDescription }
    }
    func importFile(_ url: URL) {
        guard !busy && writable else { return }
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            let handle = try FileHandle(forReadingFrom: url); defer { try? handle.close() }
            let data = try handle.read(upToCount: MobileConfiguration.maxSubscriptionBytes + 1) ?? Data()
            guard data.count <= MobileConfiguration.maxSubscriptionBytes,
                  let content = String(data: data, encoding: .utf8) else { throw MobileFailure.message("需要 10 MB 以内的 UTF-8 Clash YAML/JSON。") }
            try upsert(source: Source(name: String(url.deletingPathExtension().lastPathComponent.prefix(80)), url: ""), content: content)
            notice = "离线配置已导入。"
        } catch let error as MobileFailure { self.error = error.localizedDescription }
        catch let error as ProxyError { self.error = error.localizedDescription }
        catch { self.error = "文件导入失败，旧资料未改变。" }
    }
    func exportData() throws -> Data { try MobileConfiguration.export(library: library, node: selected) }
}
