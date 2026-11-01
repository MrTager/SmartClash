import Foundation
import Combine
import AppKit
import ProxyLogic
@MainActor
final class AppStore: ObservableObject {
    @Published var preferences = Preferences()
    @Published var nodes: [ProxyNode] = []
    @Published var statuses: [String: NodeStatus] = [:]
    @Published var selected = ""
    @Published var running = false
    @Published var tun = false
    @Published var systemProxy = false
    @Published var busy = false
    @Published var error: String?
    @Published var logs: [String] = []
    @Published var recovery = false
    @Published var subscriptionReady = false
    private let engine = CoreProcess()
    private let proxy = SystemProxy()
    private var api: CoreClient!
    private var aggregate: AggregateServer!
    private var monitor: Task<Void, Never>?
    private var standbyMonitor: Task<Void, Never>?
    private var lastGeneration = Date.distantPast
    private var probeInFlight = false
    private var failures = 0
    private var lastSwitch = Date.distantPast
    private var scanOffset = 0
    private var lastUpdate = Date()
        private let settings = Files.root.appendingPathComponent("preferences.json")
    @Published var dynamicMode = false
    @Published var routingStatus = "尚未打通"
    private var routePlan: RoutePlan?
    private var searchRound = 0
    var routeNodes: [ProxyNode] { routePlan?.candidates ?? nodes }
    var subscriptionURL: String { aggregate.address }
    var canChangeSources: Bool { !running && !busy && !recovery }
    init() {
        api = CoreClient(secret: preferences.secret)
        aggregate = AggregateServer(token: preferences.subscriptionToken)
        do {
            try Files.setup()
            if FileManager.default.fileExists(atPath: settings.path) {
                preferences = try JSONDecoder().decode(Preferences.self, from: Data(contentsOf: settings))
            }
            preferences.interval = max(1, min(3600, preferences.interval))
            try Files.write(JSONEncoder().encode(preferences), to: settings)
            api = CoreClient(secret: preferences.secret)
            aggregate = AggregateServer(token: preferences.subscriptionToken)
            nodes = SubscriptionService.cached(preferences.sources)
            if !nodes.isEmpty { try aggregate.update(nodes) }
            try aggregate.start { [weak self] message in Task { @MainActor in self?.error = message; self?.subscriptionReady = false } }
            subscriptionReady = true
            recovery = proxy.hasBackup || engine.recoveryNeeded
            systemProxy = proxy.hasBackup
            if recovery { log("发现上次运行的恢复记录，请点击“恢复网络”") }
        } catch { self.error = error.localizedDescription }
    }
    func log(_ message: String) {
        logs.insert("\(Date().formatted(date: .omitted, time: .standard))  \(message)", at: 0)
        if logs.count > 300 { logs.removeLast(logs.count - 300) }
    }
    func savePreferences() {
        do { try Files.write(JSONEncoder().encode(preferences), to: settings) }
        catch { self.error = error.localizedDescription }
    }
    func perform(_ action: @escaping () async throws -> Void) {
        guard !busy else { return }
        busy = true
        Task {
            defer { busy = false }
            do { try await action() }
            catch { self.error = error.localizedDescription; log(error.localizedDescription) }
        }
    }
    func addSource(name: String, url: String) throws {
        let text = url.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let address = URL(string: text), address.scheme == "https", address.host != nil else { throw ProxyError.message("请输入 HTTPS Clash 订阅链接") }
        guard !preferences.sources.contains(where: { $0.url == text }) else { throw ProxyError.message("该订阅已存在") }
        preferences.sources.append(Source(name: name.isEmpty ? "订阅 \(preferences.sources.count + 1)" : name, url: text))
        savePreferences()
    }
    func removeSource(_ source: Source) {
        guard canChangeSources else { return }
        preferences.sources.removeAll { $0.id == source.id }; savePreferences()
        try? FileManager.default.removeItem(at: Files.root.appendingPathComponent("subscription-\(source.id).yaml"))
        nodes = SubscriptionService.cached(preferences.sources); try? aggregate.update(nodes)
    }
    func refresh() async throws {
        // Manual refresh while stopped; running refresh uses a validated API reload in normal mode.
        // Privileged TUN config is immutable to the GUI and must be restarted explicitly.
        guard !dynamicMode else { throw ProxyError.message("智能路由运行中，请停止后更新订阅") }
        guard !tun else { throw ProxyError.message("更新订阅前请先关闭 TUN 并停止内核") }
        let update = try await SubscriptionService.update(preferences.sources, throughProxy: running)
        if running {
            let data = try Configuration.make(nodes: update.nodes, secret: preferences.secret, tun: false, routes: routePlan?.candidates, extraProxies: routePlan?.definitions ?? [])
            let temp = Files.root.appendingPathComponent("reload.json")
            try Files.write(data, to: temp)
            let test = try await Command.run(engine.executable.path, ["-t", "-d", Files.root.path, "-f", temp.path])
            guard test.code == 0 else { throw ProxyError.message("更新配置未通过内核校验，继续使用旧配置") }
            try await api.request("/configs?force=true", method: "PUT", body: ["path": temp.path])
            let available = routeNodes
            selected = available.first(where: { $0.name == selected })?.name ?? available[0].name
            try await api.select(selected, group: "SMART")
            try Files.write(data, to: Files.root.appendingPathComponent("config.json"))
        }
        nodes = update.nodes; try aggregate.update(nodes); lastUpdate = Date()
        log("订阅已合并：\(nodes.count) 个去重节点")
        for warning in update.warnings { log(warning) }
    }
    func start(tun wanted: Bool = false) async throws {
        guard !recovery else { throw ProxyError.message("请先恢复上次网络设置") }
        let data = try Configuration.make(nodes: nodes, secret: preferences.secret, tun: wanted, routes: routePlan?.candidates, extraProxies: routePlan?.definitions ?? [])
        do { try await engine.start(data: data, tun: wanted, api: api) }
        catch { recovery = engine.recoveryNeeded || proxy.hasBackup; throw error }
        running = true; tun = wanted; selected = routeNodes.first?.name ?? ""
        failures = 0; lastSwitch = .distantPast
        startMonitor(); log(wanted ? "TUN 已启动" : "代理内核已启动")
    }
    func stop() async throws {
        monitor?.cancel(); monitor = nil
        standbyMonitor?.cancel(); standbyMonitor = nil
        // Restoring the system proxy must succeed before its local endpoint is removed.
        do {
            try await proxy.restore(); systemProxy = false
            try await engine.stop()
            running = false; tun = false; selected = ""; dynamicMode = false; routePlan = nil; routingStatus = "尚未打通"; recovery = false; log("代理已停止，网络设置已恢复")
        } catch { recovery = true; throw error }
    }
    func toggleSystem() async throws {
        if systemProxy { try await proxy.restore(); systemProxy = false; return }
        if !running { try await start() }
        do { try await proxy.enable() }
        catch { recovery = proxy.hasBackup; throw error }
        systemProxy = true; log("系统 HTTP / HTTPS / SOCKS 代理已开启")
    }
    func toggleTun() async throws {
        let wanted = !tun
        let smart = dynamicMode; let plan = routePlan
        if running { try await stop() }
        dynamicMode = smart; routePlan = plan
        // stop() restores a previously enabled system proxy. TUN captures traffic independently.
        try await start(tun: wanted)
    }
    func oneClickConnect() async throws {
        if recovery { try await stop() }
        if running { try await stop() }
        if nodes.isEmpty { try await refresh() }
        searchRound = 0; dynamicMode = true; preferences.automatic = true
        routePlan = try DynamicRouter.plan(nodes: nodes, round: searchRound)
        do {
            try await start()
            try await discoverRoute()
            guard routingStatus == "已打通" else { throw ProxyError.message("本轮未找到可用链路，将在后台继续探索；可再次点击停止") }
        } catch {
            // Leave a running search available for the monitor to continue, but never claim success.
            if !running { dynamicMode = false; routePlan = nil }
            throw error
        }
    }
    private func discoverRoute() async throws {
        routingStatus = "正在探索链路"
        let deadline = Date().addingTimeInterval(24)
        for node in routeNodes {
            if Date() >= deadline { break }
            let result = try await check(node)
            if result.health == .healthy {
                try await select(node)
                if !systemProxy && !tun { try await proxy.enable(); systemProxy = true }
                routingStatus = "已打通"
                return
            }
        }
        routingStatus = "本轮未打通，等待继续探索"
    }
    private func installNextBatch() async throws {
        while probeInFlight { try await Task.sleep(nanoseconds: 100_000_000) }
        try Task.checkCancellation()
        searchRound += 1
        let next = try DynamicRouter.plan(nodes: nodes, round: searchRound)
        // Keep the old route definitions and candidate while testing the next batch.
        let retained = routeNodes.first { $0.name == selected }
        let preserved = (retained.map { [$0] } ?? []) + Array(freshStandbys.prefix(2))
        let preservedNames = Set(preserved.map(\.name))
        let candidates = preserved + next.candidates.filter { !preservedNames.contains($0.name) }
        let old = Dictionary((routePlan?.definitions ?? []).map { ($0.name, $0) }, uniquingKeysWith: { _, new in new })
        var retainedDefinitions: [ProxyNode] = []
        var visited = Set<String>()
        for candidate in preserved {
            var cursor: String? = candidate.name
            while let name = cursor, visited.insert(name).inserted, let node = old[name] {
                retainedDefinitions.append(node); cursor = node.payload["dialer-proxy"] as? String
            }
        }
        let definitions = Dictionary(retainedDefinitions.map { ($0.id, $0) }).merging(
            Dictionary(next.definitions.map { ($0.id, $0) }), uniquingKeysWith: { _, new in new })
        let data = try Configuration.make(nodes: nodes, secret: preferences.secret, tun: tun,
            routes: candidates, extraProxies: Array(definitions.values))
        try await api.request("/configs?force=true", method: "PUT", body: ["payload": String(decoding: data, as: UTF8.self)])
        if let retained { try await api.select(retained.name, group: "SMART") }
        routePlan = RoutePlan(candidates: candidates, definitions: Array(definitions.values))
        lastGeneration = Date()
    }
    private func rebuildRoutes() async throws {
        try await installNextBatch()
        try await discoverRoute()
    }
    private var freshStandbys: [ProxyNode] {
        routeNodes.filter { node in
            guard node.name != selected, let status = statuses[node.id] else { return false }
            return StandbyPolicy.usable(status.result, checked: status.checked, now: Date())
        }.sorted { (statuses[$0.id]?.result.milliseconds ?? Int.max) < (statuses[$1.id]?.result.milliseconds ?? Int.max) }
    }
    private func startStandbyMonitor() {
        standbyMonitor?.cancel()
        standbyMonitor = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                if dynamicMode && running && !busy && routingStatus == "已打通" {
                    do {
                        var pending = routeNodes.filter { $0.name != selected }.sorted {
                            (statuses[$0.id]?.checked ?? .distantPast) < (statuses[$1.id]?.checked ?? .distantPast)
                        }
                        if (pending.isEmpty && nodes.count > 1 && Date().timeIntervalSince(lastGeneration) > 5) || Date().timeIntervalSince(lastGeneration) > 60 {
                            busy = true
                            do { try await installNextBatch(); busy = false }
                            catch { busy = false; throw error }
                            pending = routeNodes.filter { $0.name != selected }
                        }
                        if let node = pending.first {
                            // Only PROBE is changed; the main monitor uses the SMART business listener.
                            _ = try await check(node)
                        }
                    } catch is CancellationError { return }
                    catch { if !Task.isCancelled { log("备用线路探测失败：\(error.localizedDescription)") } }
                }
                do { try await Task.sleep(nanoseconds: 1_000_000_000) } catch { return }
            }
        }
    }
    func select(_ node: ProxyNode) async throws {
        guard running else { throw ProxyError.message("请先启动内核") }
        try await api.select(node.name, group: "SMART")
        selected = node.name; lastSwitch = Date(); failures = 0; log("切换至 \(node.name)")
    }
    private func check(_ node: ProxyNode) async throws -> ProbeResult {
        while probeInFlight { try await Task.sleep(nanoseconds: 100_000_000) }
        try Task.checkCancellation()
        probeInFlight = true
        defer { probeInFlight = false }
        try await api.select(node.name, group: "PROBE")
        let result = try await api.probe(port: Configuration.probePort)
        try Task.checkCancellation()
        statuses[node.id] = NodeStatus(result: result, checked: Date())
        return result
    }
    func inspectCurrent(force: Bool = false) async throws {
        guard running, let current = routeNodes.first(where: { $0.name == selected }) else { return }
        // A heartbeat distinguishes a dead core from an unhealthy exit node.
        guard (try? await api.request("/version")) != nil else {
            log("内核失联，正在恢复系统网络")
            try await stop(); return
        }
        if dynamicMode && routingStatus != "已打通" { try await rebuildRoutes(); return }
        let health: ProbeResult
        if dynamicMode {
            health = try await api.probe(port: Configuration.proxyPort)
            try Task.checkCancellation()
            statuses[current.id] = NodeStatus(result: health, checked: Date())
        } else { health = try await check(current) }
        if health.health == .healthy {
            failures = 0
            if dynamicMode && routingStatus != "已打通" {
                routingStatus = "已打通"
                if !systemProxy && !tun { try await proxy.enable(); systemProxy = true }
            }
            return
        }
        if health.health == .limited || health.health == .unknown {
            failures = 0; log("OpenAI 返回限流或不确定结果，本次保留节点"); return
        }
        failures += 1
        if dynamicMode, let backup = freshStandbys.first {
            try await select(backup)
            routingStatus = "已打通"
            log("主线路失效，已切换到预热备用线路；已有请求可能需要重试")
            return
        }
        guard preferences.automatic || force else { return }
        if dynamicMode && failures >= 2 { try await rebuildRoutes(); return }
        guard force || HealthPolicy.shouldFailover(failures: failures, lastSwitch: lastSwitch, now: Date()) else { return }
        if dynamicMode { try await rebuildRoutes(); return }
        // Probe a bounded rotating batch, skipping recently failed exits.
        let alternatives = routeNodes.filter { $0.id != current.id }
        guard !alternatives.isEmpty else { log("没有备用节点"); return }
        let offset = scanOffset % alternatives.count
        let rotated = Array(alternatives[offset...] + alternatives[..<offset])
        scanOffset = (offset + 12) % alternatives.count
        let candidates = rotated.filter { node in
            guard let s = statuses[node.id] else { return true }
            return s.result.health == .healthy || Date().timeIntervalSince(s.checked) > 60
        }.prefix(12)
        var best: (ProxyNode, ProbeResult)?
        for node in candidates {
            let result = try await check(node)
            if result.health == .healthy && (best == nil || result.milliseconds < best!.1.milliseconds) { best = (node, result) }
            // Stop early when an acceptable healthy exit is found.
            if let best, best.1.milliseconds < 1500 { break }
        }
        guard let best else { log("本批备用节点均未确认可用，保留当前节点，稍后重试"); return }
        try await select(best.0)
        log("OpenAI 连通性恢复：\(best.1.milliseconds) ms；已有连接可能需要重试")
    }
    private func startMonitor() {
        startStandbyMonitor()
        monitor?.cancel()
        monitor = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                if !busy && running {
                    busy = true
                    do {
                        try await inspectCurrent()
                        if !dynamicMode && !tun && running && Date().timeIntervalSince(lastUpdate) > 3600 { try await refresh() }
                    } catch is CancellationError { busy = false; return }
                    catch { log("检测失败：\(error.localizedDescription)") }
                    busy = false
                }
                do { try await Task.sleep(nanoseconds: UInt64(dynamicMode ? 1 : max(1, preferences.interval)) * 1_000_000_000) }
                catch { return }
            }
        }
    }
    func export() {
        let panel = NSSavePanel(); panel.nameFieldStringValue = "smartclash-aggregate.yaml"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do { try Files.write(Configuration.export(nodes: nodes), to: url) }
        catch { self.error = error.localizedDescription }
    }
    func shutdown() async -> Bool {
        monitor?.cancel(); standbyMonitor?.cancel()
        // Commands have bounded network timeouts; allow any in-flight update to complete first.
        while busy { try? await Task.sleep(nanoseconds: 200_000_000) }
        do { try await stop(); return true }
        catch { self.error = error.localizedDescription; return false }
    }
}
