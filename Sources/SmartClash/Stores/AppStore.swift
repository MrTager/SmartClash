import Foundation
import Combine
import AppKit
import ProxyLogic
@MainActor
final class AppStore: ObservableObject {
    @Published var preferences = Preferences()
    @Published var nodes: [ProxyNode] = []
    private var statuses: [String: NodeStatus] = [:]
    @Published var selected = ""
    @Published var running = false
    @Published var tun = false
    @Published var systemProxy = false
    @Published var busy = false
    @Published var error: String?
    @Published var logs: [String] = []
    @Published var recovery = false
    @Published var subscriptionReady = false
    @Published var latencies: [String: NodeLatency] = [:]
    @Published var isTestingLatency = false
    @Published var latencyCompleted = 0
    @Published var latencyTotal = 0
    @Published var internetStatus = "未验证联网"
    @Published var openAIStatus = "OpenAI 尚未检测"
    @Published var measurements: [String: LinkMeasurements] = [:]
    @Published var activeMode: RoutingMode?
    @Published var activeTrace = ChainTrace.resolve(selected: "", rawNodes: [], definitions: [])
    @Published var standbyReadyCount = 0
    @Published var stopping = false
    @Published private(set) var terminating = false
    private var lifecycleReady = false
    private var startupTask: Task<Void, Never>?
    private var shutdownTask: Task<Bool, Never>?
    private var shutdownComplete = false
    @Published var ruleSnapshot: RuleSnapshot?
    @Published var rulesUpdating = false
    @Published var ruleUpdateMessage = "规则尚未下载"
    var rulesRefreshTask: Task<RuleSnapshot, Error>?
    var rulesScheduleTask: Task<Void, Never>?
    var rulesAppliedToCore = false
    var trafficMode: TrafficMode { preferences.trafficMode ?? .rule }
    var kernel: KernelKind { preferences.kernel ?? .mihomo }
    var capabilities: KernelCapabilities { kernel.capabilities }
    var ruleProviderBaseURL: String? { persistsChanges ? aggregate.ruleBaseURL : nil }
    let traffic: TrafficStore
    let activity: NetworkActivityStore
    let persistsChanges: Bool
    private var operationTask: Task<Void, Never>?
    private var stopTask: Task<Void, Never>?
    private var operationGeneration = 0
    private var monitoringRequested = false
    private var healthCheckInFlight = false
    private var routeMutationInFlight = false
    private var monitorGeneration = 0
    private var lastBackgroundMessage = ""
    private var lastBackgroundMessageAt = Date.distantPast
    private var lastOpenAIProbe = Date.distantPast
    var latencyTask: Task<Void, Never>?
    private let engine = CoreProcess()
    private let proxy = SystemProxy()
    var api: CoreClient!
    var aggregate: AggregateServer!
    private var monitor: Task<Void, Never>?
    private var standbyMonitor: Task<Void, Never>?
    private var lastGeneration = Date.distantPast
    private var probeInFlight = false
    private var failures = 0
    private var lastSwitch = Date.distantPast
    private var scanOffset = 0
    private var lastUpdate = Date()
    private let settings = Files.root.appendingPathComponent("preferences.json")
    private var outboundInterface: String?
    @Published var dynamicMode = false
    @Published var routingStatus = "尚未打通"
    private var routePlan: RoutePlan?
    private var searchRound = 0
    var routeNodes: [ProxyNode] { routePlan?.candidates ?? nodes }
    /// 用户固定（锁定）的出口节点；订阅更新后节点可能已消失，因此按 ID 现查。
    var pinnedNode: ProxyNode? {
        preferences.pinnedNodeID.flatMap { id in nodeListNodes.first { $0.id == id } }
    }
    /// 固定 / 取消固定出口节点。固定后关闭自动切换，一键打通直接使用该节点。
    func pin(_ node: ProxyNode?) {
        preferences.pinnedNodeID = node?.id
        if node != nil { preferences.automatic = false }
        savePreferences()
        if let node {
            log("已固定出口：\(node.name)；自动切换已关闭，可在节点页取消固定")
        } else {
            log("已取消固定出口，恢复自动选路")
        }
    }
    var visualizationNodes: [ProxyNode] {
        var seen = Set<String>()
        return (nodes + (routePlan?.definitions ?? []) + routeNodes).filter { seen.insert($0.name).inserted }
    }
    var subscriptionURL: String { aggregate.address }
    var canChangeSources: Bool { !terminating && !running && !recovery }
    init(loadFromDisk: Bool = true) {
        var bootstrapError: Error?
        if loadFromDisk {
            do { try GuardianClient.shared.bootstrap() }
            catch { bootstrapError = error }
        }
        persistsChanges = loadFromDisk
        let mayReadDisk = loadFromDisk && bootstrapError == nil
        traffic = TrafficStore(filename: mayReadDisk ? Files.root.appendingPathComponent("traffic-history.json") : nil)
        activity = NetworkActivityStore(geo: GeoIPService(cacheURL: mayReadDisk ? Files.root.appendingPathComponent("geoip-cache.json") : nil))
        api = CoreClient(secret: preferences.secret)
        aggregate = AggregateServer(token: preferences.subscriptionToken)
        guard loadFromDisk else { return }
        if let bootstrapError {
            recovery = true
            self.error = "生命周期守护初始化失败：\(bootstrapError.localizedDescription)"
            return
        }
        startupTask = Task { @MainActor [weak self] in
            guard let self else { return }
            defer { self.startupTask = nil }
            do {
                try self.checkNotTerminating()
                try await PrivilegedClient.shared.reconcileAtStartup()
                try self.checkNotTerminating()
                try self.initializeFromDisk()
                self.lifecycleReady = true
            } catch {
                self.recovery = true
                let message = "启动对账未完成：\(error.localizedDescription)"
                self.error = message; self.log(message)
            }
        }
    }
    private func initializeFromDisk() throws {
        try checkNotTerminating()
        if FileManager.default.fileExists(atPath: settings.path) {
            preferences = try JSONDecoder().decode(Preferences.self, from: Data(contentsOf: settings))
        }
        preferences.interval = max(1, min(3600, preferences.interval))
        // 代理端口非法或与保留端口冲突时回退默认值，避免历史脏数据卡死启动。
        if let saved = preferences.proxyPort, Configuration.validateProxyPort(saved) != nil {
            preferences.proxyPort = Configuration.defaultProxyPort
            log("已保存的代理端口不可用，已回退为 \(Configuration.defaultProxyPort)")
        }
        let port = preferences.proxyPort ?? Configuration.defaultProxyPort
        preferences.proxyPort = port
        Configuration.proxyPort = port
        engine.configure(kind: preferences.kernel ?? .mihomo)
        try Files.write(JSONEncoder().encode(preferences), to: settings)
        api = CoreClient(secret: preferences.secret)
        aggregate = AggregateServer(token: preferences.subscriptionToken)
        ruleSnapshot = RuleSubscriptionService.cached(at: Files.root.appendingPathComponent("routing-rules.json"))
        try aggregate.updateRules(ruleSnapshot ?? RuleSnapshot(modules: [:]))
        if let snapshot = ruleSnapshot { ruleUpdateMessage = "已载入 \(snapshot.count) 条缓存规则" }
        nodes = SubscriptionService.cached(preferences.sources)
        if let data = try? Data(contentsOf: Files.root.appendingPathComponent("latencies.json")),
           let saved = try? JSONDecoder().decode([String: NodeLatency].self, from: data) {
            let ids = Set(nodes.map(\.id)); latencies = saved.filter { ids.contains($0.key) }
        }
        if let data = try? Data(contentsOf: Files.root.appendingPathComponent("link-measurements.json")),
           let saved = try? JSONDecoder().decode([String: LinkMeasurements].self, from: data) { measurements = saved }
        if !nodes.isEmpty { try aggregate.update(nodes) }
        try aggregate.start { [weak self] message in Task { @MainActor in
            guard let self, !self.terminating else { return }
            self.error = message; self.subscriptionReady = false
        } }
        subscriptionReady = true
        recovery = proxy.hasBackup || engine.recoveryNeeded
        systemProxy = proxy.hasBackup
        if recovery { log("发现上次运行的恢复记录，请点击“恢复网络”") }
        startRuleSchedule()
    }
    func checkNotTerminating() throws {
        guard !terminating else { throw CancellationError() }
        try Task.checkCancellation()
    }
    func checkLifecycle() throws {
        try checkNotTerminating()
        guard !persistsChanges || lifecycleReady else { throw ProxyError.message("生命周期启动对账尚未完成，暂不接受操作") }
    }
    func log(_ message: String) {
        logs.insert("\(Date().formatted(date: .omitted, time: .standard))  \(message)", at: 0)
        if logs.count > 300 { logs.removeLast(logs.count - 300) }
    }
    func savePreferences() {
        guard persistsChanges else { return }
        do { try Files.write(JSONEncoder().encode(preferences), to: settings) }
        catch { self.error = error.localizedDescription }
    }
    @discardableResult
    func perform(_ action: @escaping @MainActor () async throws -> Void) -> Task<Void, Never> {
        guard !terminating else { return Task {} }
        guard !persistsChanges || lifecycleReady else {
            error = "生命周期启动对账尚未完成，暂不接受操作"
            return Task {}
        }
        let previous = operationTask
        previous?.cancel()
        let restoring = stopTask
        operationGeneration += 1
        let generation = operationGeneration
        busy = true
        let task = Task {
            // Finish cancellation/rollback before another action changes the core.
            await previous?.value
            await restoring?.value
            do {
                try checkLifecycle()
                await pauseMonitoring()
                try checkLifecycle()
                try await action()
            } catch {
                if !Task.isCancelled && !(error is CancellationError) {
                    self.error = error.localizedDescription; log(error.localizedDescription)
                }
            }
            guard generation == operationGeneration else { return }
            operationTask = nil
            busy = false
            if running && monitoringRequested && !stopping && !terminating { startMonitor() }
        }
        operationTask = task
        return task
    }
    private func pauseMonitoring() async {
        monitorGeneration += 1
        let health = monitor; let standby = standbyMonitor
        monitor = nil; standbyMonitor = nil
        health?.cancel(); standby?.cancel()
        await health?.value; await standby?.value
        healthCheckInFlight = false
    }
    func requestStop() {
        if terminating {
            Task { _ = await shutdown() }
            return
        }
        guard !persistsChanges || lifecycleReady else { return }
        operationTask?.cancel(); latencyTask?.cancel()
        monitor?.cancel(); standbyMonitor?.cancel()
        guard !stopping else { return }
        stopping = true
        monitoringRequested = false
        let previous = operationTask
        operationTask = nil
        operationGeneration += 1
        let generation = operationGeneration
        busy = true
        stopTask = Task {
            await previous?.value
            await pauseMonitoring()
            defer {
                stopping = false; stopTask = nil
                if generation == operationGeneration { busy = false }
            }
            do { try await stop() }
            catch { self.error = error.localizedDescription }
        }
    }
    func logBackground(_ message: String) {
        guard message != lastBackgroundMessage || Date().timeIntervalSince(lastBackgroundMessageAt) >= 60 else { return }
        lastBackgroundMessage = message; lastBackgroundMessageAt = Date(); log(message)
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
        try checkLifecycle()
        // Manual refresh while stopped; running refresh uses a validated API reload in normal mode.
        // Privileged TUN config is immutable to the GUI and must be restarted explicitly.
        guard !dynamicMode else { throw ProxyError.message("智能路由运行中，请停止后更新订阅") }
        guard !tun else { throw ProxyError.message("更新订阅前请先关闭 TUN 并停止内核") }
        let update = try await SubscriptionService.update(preferences.sources, throughProxy: running)
        try checkLifecycle()
        nodes = update.nodes; try aggregate.update(nodes); lastUpdate = Date()
        for warning in update.warnings { log(warning) }
        if running { try await reloadRunningCore() }
        log("订阅已合并：\(nodes.count) 个去重节点")
    }
    func start(tun wanted: Bool = false, preferredName requestedName: String? = nil, purpose: CoreStartPurpose = .connection) async throws {
        try checkLifecycle()
        guard persistsChanges else { throw ProxyError.message("离线验收实例不启动真实内核") }
        guard !recovery else { throw ProxyError.message("请先恢复上次网络设置") }
        if wanted && !kernel.capabilities.tun { throw ProxyError.message("\(kernel.title) 内核不支持 TUN，请改用 Mihomo 或 sing-box") }
        if trafficMode == .rule && purpose == .connection { try await ensureRulesReady() }
        let preferred = NodeRanking.connectionCandidates(routeNodes, latencies: latencies).first { kernel.supports($0) }
        let preferredName = requestedName ?? preferred?.name
        outboundInterface = try await NetworkEnvironment.defaultInterface(forTun: wanted)
        try checkLifecycle()
        let built = try KernelConfiguration.make(kind: kernel, nodes: nodes, secret: preferences.secret, tun: wanted,
            routes: routePlan?.candidates, extraProxies: routePlan?.definitions ?? [],
            outboundInterface: outboundInterface, preferredName: preferredName,
            trafficMode: trafficMode, ruleProviderBaseURL: ruleProviderBaseURL)
        for warning in built.warnings { log(warning) }
        do {
            try await engine.start(data: built.data, tun: wanted, api: api, kind: kernel)
            try checkLifecycle()
        }
        catch { recovery = engine.recoveryNeeded || proxy.hasBackup; throw error }
        running = true; tun = wanted
        rulesAppliedToCore = false
        if kernel.capabilities.rules == .provider, ruleProviderBaseURL != nil, ruleSnapshot != nil {
            do { try await api.updateRuleProviders(); rulesAppliedToCore = ruleSnapshot != nil }
            catch {
                if trafficMode == .rule { try? await stop(); throw error }
                logBackground("规则暂未载入内核，\(trafficMode.title)继续运行")
            }
        } else if kernel.capabilities.rules != .provider {
            rulesAppliedToCore = true
        }
        try checkLifecycle()
        selected = routeNodes.first(where: { $0.name == preferredName })?.name ?? routeNodes.first?.name ?? ""
        internetStatus = "内核已启动，尚未验证出网"
        publishRoute()
        let runID = UUID().uuidString
        if kernel == .xray {
            traffic.startPolling(executable: engine.executable, runID: runID)
        } else {
            traffic.start(client: api, runID: runID)
        }
        if kernel.capabilities.connections {
            activity.start(client: api, traffic: traffic, runID: runID)
        } else {
            activity.stop()
        }
        failures = 0; lastSwitch = .distantPast
        if purpose.monitorsConnections { startMonitor() }
        log(wanted ? "TUN 已启动" : (purpose == .latencyTest ? "仅测速内核已启动，未接管系统网络" : "代理内核已启动"))
    }
    func stop() async throws {
        monitoringRequested = false; monitorGeneration += 1
        monitor?.cancel(); monitor = nil
        standbyMonitor?.cancel(); standbyMonitor = nil
        guard persistsChanges else {
            running = false; tun = false; systemProxy = false; dynamicMode = false
            traffic.stop(); activity.stop(); return
        }
        // Restore the system proxy before removing its local endpoint, but never let a
        // failed proxy restore skip stopping the privileged core: a surviving TUN keeps
        // hijacking all system traffic while the UI would claim the network is restored.
        var errors: [String] = []
        do { try await proxy.restore() } catch { errors.append(error.localizedDescription) }
        do { try await engine.stop() } catch { errors.append(error.localizedDescription) }
        if GuardianClient.shared.hasChildren {
            do { try await GuardianClient.shared.stopAll() } catch { errors.append(error.localizedDescription) }
        }
        traffic.stop()
        activity.stop()
        systemProxy = proxy.hasBackup
        running = false; tun = false; selected = ""; dynamicMode = false; routePlan = nil
        routingStatus = "尚未打通"; internetStatus = "未连接"; outboundInterface = nil
        recovery = engine.recoveryNeeded || proxy.hasBackup || GuardianClient.shared.hasChildren || !errors.isEmpty
        activeMode = nil; publishRoute()
        if errors.isEmpty { log("代理已停止，网络设置已恢复") }
        else { log("停止未完全成功：\(errors.joined(separator: "；"))") }
        if !errors.isEmpty { throw ProxyError.message(errors.joined(separator: "；")) }
    }
    /// 带现场恢复的内核重启：保留智能路由计划与当前出口，TUN 重启后复验出网，
    /// 原本开启的系统代理会自动重新指向新端口。与 Windows 版 restart_core 语义一致。
    func restartCore(preferredName: String? = nil) async throws {
        try checkLifecycle()
        guard running else { return }
        let wasTun = tun
        let wasSystemProxy = systemProxy
        let wasDynamic = dynamicMode
        let plan = routePlan
        let previous = preferredName ?? selected
        try await stop()
        dynamicMode = wasDynamic; routePlan = plan
        try await start(tun: wasTun, preferredName: previous)
        if wasTun {
            let result = try await verifyTunEgress()
            internetStatus = "TUN 出网已验证 · \(result.milliseconds) ms"
            routingStatus = "已打通"
        }
        if wasSystemProxy {
            try await proxy.enable(); systemProxy = true
        }
        startMonitor()
    }
    /// 修改本地混合代理端口。运行中会自动重启内核使新端口生效；失败时回滚。
    func changeProxyPort(_ port: Int) async throws {
        try checkLifecycle()
        let current = Configuration.proxyPort
        guard current != port else { return }
        if let message = Configuration.validateProxyPort(port) { throw ProxyError.message(message) }
        preferences.proxyPort = port
        Configuration.proxyPort = port
        savePreferences()
        log("本地代理端口改为 \(port)")
        guard running else { return }
        do { try await restartCore(preferredName: selected) }
        catch {
            // 新端口启动失败时回滚到原端口，避免界面与已停止的内核状态不一致。
            preferences.proxyPort = current
            Configuration.proxyPort = current
            savePreferences()
            log("端口 \(port) 启动失败，已回滚为 \(current)")
            do { try await restartCore(preferredName: selected) }
            catch { recovery = true; log("端口修改回滚失败，请停止并恢复网络") }
            throw ProxyError.message("使用端口 \(port) 重启内核失败：\(error.localizedDescription)")
        }
        log("内核已用新端口重启")
    }
    /// 订阅刷新后把合并结果应用到运行中的内核：
    /// Mihomo / sing-box 走 Clash 控制接口热重载完整配置；Xray 只能重启生效。
    func reloadRunningCore() async throws {
        try checkLifecycle()
        guard running else { return }
        if kernel.capabilities.liveSwitch {
            let built = try KernelConfiguration.make(kind: kernel, nodes: nodes, secret: preferences.secret, tun: tun,
                routes: routePlan?.candidates, extraProxies: routePlan?.definitions ?? [],
                outboundInterface: outboundInterface, preferredName: selected, trafficMode: trafficMode, ruleProviderBaseURL: ruleProviderBaseURL)
            try await engine.validate(data: built.data)
            try await api.request("/configs?force=true", method: "PUT", body: ["payload": String(decoding: built.data, as: UTF8.self)])
            if !selected.isEmpty { try? await api.select(selected, group: "SMART") }
        } else {
            try await restartCore(preferredName: selected)
        }
        rulesAppliedToCore = false
        if running && kernel.capabilities.rules == .provider && ruleProviderBaseURL != nil && ruleSnapshot != nil {
            do { try await api.updateRuleProviders(); rulesAppliedToCore = true }
            catch { logBackground("规则暂未载入内核，\(trafficMode.title)继续运行") }
        }
        publishRoute()
        log("运行中的内核已应用最新订阅（\(nodes.count) 个节点）")
    }
    /// 切换前的节点预检：用临时单节点内核完成一次 HTTPS 探测，不影响当前业务出口。
    func latencySample(_ node: ProxyNode) async -> NodeLatency {
        guard !terminating, persistsChanges, lifecycleReady else { return NodeLatency(milliseconds: nil, failure: "生命周期尚未就绪或正在退出") }
        return await EphemeralProbe.latency(kind: kernel, node: node)
    }
    func toggleSystem() async throws {
        try checkLifecycle()
        if systemProxy { try await proxy.restore(); systemProxy = false; dynamicMode = false; routingStatus = "系统代理已关闭，内核继续运行"; return }
        let startedHere = !running
        do {
            if !running { try await start() }
            if trafficMode != .direct { try await ensureReachableRoute() }
            try await proxy.enable()
        } catch {
            if startedHere { try? await stop() }
            recovery = proxy.hasBackup || engine.recoveryNeeded
            throw error
        }
        systemProxy = true
        if trafficMode == .direct { routingStatus = "直连模式"; internetStatus = "直连模式已开启" }
        startMonitor(); log("系统 HTTP / HTTPS / SOCKS 代理已开启")
    }
    func toggleTun() async throws {
        try checkLifecycle()
        let wanted = !tun
        let smart = dynamicMode
        if !wanted {
            let plan = routePlan; let previous = selected
            try await stop(); dynamicMode = false; routePlan = plan
            try await start(preferredName: previous)
            return
        }
        // Prove a single-hop route while still unprivileged, before capturing OS traffic.
        do {
            if !running { try await start() }
            if trafficMode != .direct { try await ensureReachableRoute() }
            let verified = selected; let plan = routePlan
            try await stop(); dynamicMode = smart; routePlan = plan
            try await start(tun: true, preferredName: verified)
            if trafficMode == .direct {
                internetStatus = "TUN 直连模式已开启"; routingStatus = "直连模式"; return
            }
            let result = try await verifyTunEgress()
            internetStatus = "TUN 出网已验证 · \(result.milliseconds) ms"
            routingStatus = "已打通"
        } catch is CancellationError {
            // An explicit stop owns the rollback; never report it as a TUN failure.
            try? await stop()
            throw CancellationError()
        } catch {
            let reason = error.localizedDescription
            do { try await stop() }
            catch let stopError {
                recovery = true
                log("网络自动恢复未完成：\(stopError.localizedDescription)")
                throw ProxyError.message(TunVerificationPolicy.recoveryMessage(reason, stopError: stopError.localizedDescription))
            }
            throw ProxyError.message(reason + "；已自动恢复网络")
        }
    }

    /// Verify the business listener after takeover. The privileged core is a brand new
    /// process: installing the default route, starting DNS hijacking and completing the
    /// first proxy handshake all take time. One 6-second probe used to roll back a
    /// healthy TUN whenever the exit node hiccuped, making TUN impossible to enable.
    private func verifyTunEgress() async throws -> ProbeResult {
        let started = Date()
        var attempts = 0
        var result = ProbeResult(health: .offline, milliseconds: 0, detail: "尚未探测")
        while true {
            try Task.checkCancellation()
            attempts += 1
            do { result = try await api.internetProbe(port: Configuration.proxyPort) }
            catch is CancellationError { throw CancellationError() }
            catch {
                result = ProbeResult(health: .offline, milliseconds: 0, detail: error.localizedDescription)
                log("TUN 出网探测 \(attempts) 异常：\(error.localizedDescription)")
            }
            if result.health == .healthy {
                if attempts > 1 { log("TUN 路由收敛后第 \(attempts) 次出网验证通过") }
                return result
            }
            guard TunVerificationPolicy.shouldRetry(attempts, elapsed: Date().timeIntervalSince(started)) else { break }
            log("TUN 出网探测 \(attempts) 未通过：\(result.detail)")
            internetStatus = TunVerificationPolicy.progress(attempts)
            try await Task.sleep(nanoseconds: UInt64(TunVerificationPolicy.retryDelay * 1_000_000_000))
        }
        throw ProxyError.message(TunVerificationPolicy.failureMessage(attempts: attempts, last: result.detail))
    }
    func oneClickConnect() async throws {
        try checkLifecycle()
        if recovery { try await stop() }
        if running { try await stop() }
        if trafficMode == .direct {
            try await start()
            do { try await proxy.enable(); systemProxy = true; startMonitor() }
            catch { try? await stop(); throw error }
            routingStatus = "直连模式"; internetStatus = "直连模式已开启"; return
        }
        // 用户固定了出口：直接连该节点并验证出网，不再自动搜索与切换。
        if let pinned = pinnedNode { return try await connectPinned(pinned) }
        if nodes.isEmpty { try await refresh() }
        searchRound = 0; dynamicMode = kernel == .mihomo; preferences.automatic = true
        // Start with the complete single-hop pool; never capture traffic on an untested chain.
        routePlan = RoutePlan(candidates: nodes, definitions: [])
        do {
            try await start()
            try await discoverRoute()
            guard routingStatus == "已打通" else { throw ProxyError.message("出网未通过验证，未开启新的系统代理") }
        } catch {
            try? await stop()
            throw error
        }
    }
    /// 固定节点的连接路径：不跑自动搜索，只用该节点验证 HTTPS 出网后开启系统代理。
    private func connectPinned(_ node: ProxyNode) async throws {
        guard kernel.supports(node) else {
            throw ProxyError.message("固定节点不被 \(kernel.title) 内核支持，请更换节点或内核")
        }
        dynamicMode = false
        routePlan = RoutePlan(candidates: [node], definitions: [])
        do {
            try await start(preferredName: node.name)
            let result = try await api.internetProbe(port: Configuration.proxyPort)
            guard result.health == .healthy else {
                throw ProxyError.message("固定节点未通过 HTTPS 出网验证：\(result.detail)")
            }
            try await proxy.enable(); systemProxy = true
            internetStatus = "固定节点出网已验证 · \(result.milliseconds) ms"
            routingStatus = "已打通"; activeMode = preferences.routingMode
            log("已连接固定节点：\(node.name)")
        } catch {
            try? await stop()
            throw error
        }
    }
    private func discoverRoute() async throws {
        try await ensureReachableRoute()
        if !systemProxy && !tun { try await proxy.enable(); systemProxy = true }
        routingStatus = "已打通"
    }
    func ensureReachableRoute(preferCurrent: Bool = true) async throws {
        try checkLifecycle()
        let mode = preferences.routingMode ?? .lowLatency
        routingStatus = "正在验证\(mode.title)"
        var candidates = NodeRanking.connectionCandidates(nodeListNodes, latencies: latencies)
        if preferCurrent, let index = candidates.firstIndex(where: { $0.name == selected && latencies[$0.id]?.reachable == true && latencies[$0.id]?.isFresh() == true }) {
            candidates.insert(candidates.remove(at: index), at: 0)
        }
        candidates = NodeRanking.preferNonHongKong(candidates)
        let deadline = Date().addingTimeInterval(30)
        // Recheck a fresh known success before launching another batch of tests.
        if mode == .lowLatency, let current = candidates.first, latencies[current.id]?.reachable == true,
           latencies[current.id]?.isFresh() == true {
            try await select(current)
            let result = try await withProbe(current) { port in try await api.internetProbe(port: port) }
            if result.health == .healthy {
                internetStatus = "HTTPS 出网已验证 · \(result.milliseconds) ms"; routingStatus = "已打通"; activeMode = mode; return
            }
            latencies[current.id] = NodeLatency(milliseconds: nil, failure: "业务出网未通过")
        }
        var offset = 0
        var speedAttempts = 0
        var speedTested = Set<String>()
        var fallbackStarted = false
        while offset < candidates.count && Date() < deadline {
            try Task.checkCancellation()
            // Do not let a faster fallback exit win a mixed-region probe batch.
            let fallbackTier = NodeRanking.isHongKong(candidates[offset])
            if fallbackTier && !fallbackStarted { speedAttempts = 0; fallbackStarted = true }
            let batch = Array(candidates.dropFirst(offset).prefix(6).prefix { NodeRanking.isHongKong($0) == fallbackTier })
            offset += batch.count
            await measureLatencies(batch)
            try Task.checkCancellation()
            let reachable = NodeRanking.sorted(batch, latencies: latencies).filter { latencies[$0.id]?.reachable == true }
            if mode == .ai {
                for node in reachable {
                    if Date() >= deadline { break }
                    if try await check(node).health == .healthy { break }
                }
            } else if mode == .highSpeed {
                routingStatus = "正在抽测下载吞吐（每组最多 3 MiB）"
                for node in reachable {
                    guard speedAttempts < 3, Date() < deadline else { break }
                    speedAttempts += 1
                    speedTested.insert(node.id)
                    let sample = try await withProbe(node) { port in try await api.throughput(port: port) }
                    var measured = measurements[node.id] ?? LinkMeasurements(); measured.throughput = sample
                    measurements[node.id] = measured
                }
            }
            let eligible = mode == .highSpeed ? reachable.filter { speedTested.contains($0.id) } : reachable
            for node in ModeRanking.best(eligible, mode: mode, latencies: latencies, measurements: measurements) {
                let previous = routeNodes.first(where: { $0.name == selected })
                try await select(node)
                let result = try await withProbe(node) { port in try await api.internetProbe(port: port) }
                if result.health == .healthy {
                    internetStatus = "HTTPS 出网已验证 · \(result.milliseconds) ms"
                    routingStatus = "已打通"; activeMode = mode; saveLatencies(); saveMeasurements(); return
                }
                if let previous { try await select(previous) }
                latencies[node.id] = NodeLatency(milliseconds: nil, failure: "业务出网未通过")
                if Date() >= deadline { break }
            }
            if mode == .highSpeed && speedAttempts >= 3 {
                guard !fallbackTier, let fallback = candidates.indices.dropFirst(offset).first(where: { NodeRanking.isHongKong(candidates[$0]) }) else { break }
                offset = fallback; speedAttempts = 0
            }
        }
        saveLatencies(); saveMeasurements(); routingStatus = "\(mode.title)未通过验证"
        throw ProxyError.message("本轮没有通过\(mode.title)验证的线路；未开启新的系统代理，请检查订阅或尝试其他模式")
    }
    func applyRoutingMode() async throws {
        try checkLifecycle()
        guard running else { throw ProxyError.message("请先连接，或启动本机内核") }
        guard trafficMode != .direct else { throw ProxyError.message("直连模式不使用代理选路，请先切换流量模式") }
        let previousStatus = routingStatus
        do { try await ensureReachableRoute(preferCurrent: false); startMonitor() }
        catch { routingStatus = previousStatus; throw error }
    }
    func saveMeasurements() {
        guard persistsChanges else { return }
        if let data = try? JSONEncoder().encode(measurements) { try? Files.write(data, to: Files.root.appendingPathComponent("link-measurements.json")) }
    }
    private func installNextBatch() async throws {
        // sing-box 的 SMART 组已包含全部节点，Xray 只能重启应用新配置；动态多跳仅保留给 Mihomo。
        guard kernel == .mihomo else { return }
        while probeInFlight || routeMutationInFlight { try await Task.sleep(nanoseconds: 100_000_000) }
        routeMutationInFlight = true; defer { routeMutationInFlight = false }
        try Task.checkCancellation()
        searchRound += 1
        let ordered = NodeRanking.connectionCandidates(nodes, latencies: latencies)
        let next = try DynamicRouter.plan(nodes: ordered, round: searchRound, allowChains: preferences.exploreMultihop == true)
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
        let definitions = Dictionary(uniqueKeysWithValues: retainedDefinitions.map { ($0.id, $0) }).merging(
            Dictionary(uniqueKeysWithValues: next.definitions.map { ($0.id, $0) }), uniquingKeysWith: { _, new in new })
        let data = try Configuration.make(nodes: nodes, secret: preferences.secret, tun: tun,
            routes: candidates, extraProxies: Array(definitions.values), outboundInterface: outboundInterface, preferredName: selected, trafficMode: trafficMode, ruleProviderBaseURL: ruleProviderBaseURL)
        try await api.request("/configs?force=true", method: "PUT", body: ["payload": String(decoding: data, as: UTF8.self)])
        if let retained { try await api.select(retained.name, group: "SMART") }
        routePlan = RoutePlan(candidates: candidates, definitions: Array(definitions.values))
        lastGeneration = Date()
        publishRoute()
    }
    private func rebuildRoutes() async throws {
        try await installNextBatch()
        try await discoverRoute()
    }
    private var freshStandbys: [ProxyNode] {
        NodeRanking.preferNonHongKong(routeNodes.filter { node in
            guard node.name != selected, let status = statuses[node.id] else { return false }
            return StandbyPolicy.usable(status.result, checked: status.checked, now: Date())
        }.sorted {
            if activeMode == .highSpeed {
                let left = measurements[$0.id]?.throughput?.bytesPerSecond ?? 0
                let right = measurements[$1.id]?.throughput?.bytesPerSecond ?? 0
                if left != right { return left > right }
            }
            return (statuses[$0.id]?.result.milliseconds ?? Int.max) < (statuses[$1.id]?.result.milliseconds ?? Int.max)
        })
    }
    private func startStandbyMonitor() {
        guard !terminating else { return }
        // 备用线路轮询依赖运行时追加配置；sing-box / Xray 使用即时选路，不做后台预生成。
        guard kernel == .mihomo else { return }
        standbyMonitor?.cancel()
        standbyMonitor = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                if trafficMode != .direct && dynamicMode && running && !busy && !isTestingLatency && routingStatus == "已打通" {
                    do {
                        var pending = routeNodes.filter { $0.name != self.selected }.sorted {
                            (self.statuses[$0.id]?.checked ?? .distantPast) < (self.statuses[$1.id]?.checked ?? .distantPast)
                        }
                        if !healthCheckInFlight && !routeMutationInFlight && ((pending.isEmpty && nodes.count > 1 && Date().timeIntervalSince(lastGeneration) > 5) || Date().timeIntervalSince(lastGeneration) > 60) {
                            try await installNextBatch()
                            pending = routeNodes.filter { $0.name != self.selected }
                        }
                        if let node = pending.first {
                            // Only PROBE is changed; the main monitor uses the SMART business listener.
                            _ = try await check(node, openAI: activeMode == .ai)
                        }
                    } catch is CancellationError { return }
                    catch { if !Task.isCancelled { logBackground("备用线路探测失败：\(error.localizedDescription)") } }
                }
                do { try await Task.sleep(nanoseconds: 1_000_000_000) } catch { return }
            }
        }
    }
    func select(_ node: ProxyNode) async throws {
        try checkLifecycle()
        guard running else { throw ProxyError.message("请先启动内核") }
        if kernel.capabilities.liveSwitch {
            if let plan = routePlan, !plan.candidates.contains(where: { $0.id == node.id }) {
                let candidates = plan.candidates + [node]
                if kernel == .mihomo {
                    let built = try KernelConfiguration.make(kind: kernel, nodes: nodes, secret: preferences.secret, tun: tun,
                        routes: candidates, extraProxies: plan.definitions, outboundInterface: outboundInterface,
                        preferredName: selected, trafficMode: trafficMode, ruleProviderBaseURL: ruleProviderBaseURL)
                    try await engine.validate(data: built.data)
                    try await api.request("/configs?force=true", method: "PUT", body: ["payload": String(decoding: built.data, as: UTF8.self)])
                }
                routePlan = RoutePlan(candidates: candidates, definitions: plan.definitions)
            }
            try await api.select(node.name, group: "SMART")
        } else {
            if let plan = routePlan, !plan.candidates.contains(where: { $0.id == node.id }) {
                routePlan = RoutePlan(candidates: plan.candidates + [node], definitions: plan.definitions)
            }
            // Xray 没有节点选择接口，只能带着新出口重启内核。
            try await restartCore(preferredName: node.name)
        }
        selected = node.name; lastSwitch = Date(); failures = 0; log("切换至 \(node.name)")
        publishRoute()
    }
    func publishRoute() {
        let trace = ChainTrace.resolve(selected: running && trafficMode != .direct ? selected : "", rawNodes: nodes, definitions: routePlan?.definitions ?? [])
        if activeTrace != trace { activeTrace = trace }
        let known = visualizationNodes
        activity.configure(nodes: known, active: trace.hops.compactMap { hop in known.first { $0.id == hop.id } })
        let ready = freshStandbys.count
        if standbyReadyCount != ready { standbyReadyCount = ready }
    }
    func selectVerified(_ node: ProxyNode) async throws {
        try checkLifecycle()
        guard running else { throw ProxyError.message("请先启动内核或运行一次延迟测试") }
        guard trafficMode != .direct else { throw ProxyError.message("直连模式不使用代理节点，请先切换流量模式") }
        let previous = routeNodes.first(where: { $0.name == selected })
        let sample = await latencySample(node)
        try Task.checkCancellation()
        latencies[node.id] = sample; saveLatencies()
        guard sample.reachable else { throw ProxyError.message("该节点本次 HTTPS 测试未通过，已保留原业务出口") }
        try await select(node)
        let result = try await withProbe(node) { port in try await api.internetProbe(port: port) }
        guard result.health == .healthy else {
            if let previous { try await select(previous) }
            throw ProxyError.message("切换后的业务出网验证未通过，已尝试恢复原出口")
        }
        internetStatus = "HTTPS 出网已验证 · \(result.milliseconds) ms"
        openAIStatus = "OpenAI 尚未检测"
        startMonitor()
    }
    func withProbe<T>(_ node: ProxyNode, operation: (Int) async throws -> T) async throws -> T {
        try checkLifecycle()
        // Xray 没有 Clash 控制接口，无法切换 PROBE 组；改用临时单节点内核完成探测。
        if kernel == .xray {
            try Task.checkCancellation()
            return try await EphemeralProbe.run(kind: kernel, node: node, operation: operation)
        }
        while probeInFlight { try await Task.sleep(nanoseconds: 100_000_000) }
        try Task.checkCancellation()
        probeInFlight = true
        defer { probeInFlight = false }
        try await api.select(node.name, group: "PROBE")
        try Task.checkCancellation()
        return try await operation(Configuration.probePort)
    }
    func check(_ node: ProxyNode, openAI: Bool = true) async throws -> ProbeResult {
        let result = try await withProbe(node) { _ in
            if openAI { return try await api.probe(port: Configuration.probePort) }
            return try await api.internetProbe(port: Configuration.probePort)
        }
        statuses[node.id] = NodeStatus(result: result, checked: Date())
        if openAI {
            var measured = measurements[node.id] ?? LinkMeasurements()
            measured.openAIReachable = result.health == .healthy; measured.openAIChecked = Date()
            let previous = measurements[node.id]
            if busy || previous?.openAIReachable != measured.openAIReachable || Date().timeIntervalSince(previous?.openAIChecked ?? .distantPast) >= 60 {
                measurements[node.id] = measured
            }
        }
        publishRoute()
        return result
    }
    func inspectCurrent(force: Bool = false) async throws {
        try checkLifecycle()
        guard running else { return }
        // A heartbeat distinguishes a dead core from an unhealthy exit node.
        let heartbeat = try? await api.request("/version")
        try Task.checkCancellation()
        guard heartbeat != nil else {
            log("内核失联，正在恢复系统网络")
            try await stop(); return
        }
        if trafficMode == .direct { return }
        guard let current = routeNodes.first(where: { $0.name == selected }) else { return }
        if dynamicMode && routingStatus != "已打通" { try await rebuildRoutes(); return }
        let internet = try await withProbe(current) { _ in try await api.internetProbe(port: Configuration.probePort) }
        let networkText = internet.health == .healthy ? "HTTPS 出网正常" : "当前线路出网失败，正在检查备用"
        if internetStatus != networkText { internetStatus = networkText }
        let health: ProbeResult
        if internet.health == .healthy && (activeMode == .ai || Date().timeIntervalSince(lastOpenAIProbe) >= 30) {
            let ai = try await withProbe(current) { _ in try await api.probe(port: Configuration.probePort) }
            lastOpenAIProbe = Date()
            health = activeMode == .ai ? ai : internet
            let text = "OpenAI：\(ai.detail)"; if openAIStatus != text { openAIStatus = text }
        } else if internet.health == .healthy { health = internet
        } else { health = internet; if openAIStatus != "OpenAI：等待网络恢复" { openAIStatus = "OpenAI：等待网络恢复" } }
        try Task.checkCancellation()
        statuses[current.id] = NodeStatus(result: health, checked: Date())
        if health.health == .healthy {
            failures = 0
            if dynamicMode && routingStatus != "已打通" {
                routingStatus = "已打通"
                if !systemProxy && !tun { try await proxy.enable(); systemProxy = true }
            }
            return
        }
        if health.health == .limited || health.health == .unknown {
            failures = 0; logBackground("OpenAI 返回限流或不确定结果，本次保留节点"); return
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
        let candidates = NodeRanking.preferNonHongKong(rotated.filter { node in
            guard let s = statuses[node.id] else { return true }
            return s.result.health == .healthy || Date().timeIntervalSince(s.checked) > 60
        }).prefix(12)
        var best: (ProxyNode, ProbeResult)?
        for node in candidates {
            if NodeRanking.isHongKong(node), let best, !NodeRanking.isHongKong(best.0) { break }
            let result = try await check(node, openAI: activeMode == .ai)
            if result.health == .healthy && (best == nil ||
                (NodeRanking.isHongKong(node) == NodeRanking.isHongKong(best!.0) && result.milliseconds < best!.1.milliseconds)) { best = (node, result) }
            // Stop early when an acceptable healthy exit is found.
            if let best, best.1.milliseconds < 1500 { break }
        }
        guard let best else { log("本批备用节点均未确认可用，保留当前节点，稍后重试"); return }
        try await select(best.0)
        log("OpenAI 连通性恢复：\(best.1.milliseconds) ms；已有连接可能需要重试")
    }
    private func startMonitor() {
        guard !terminating else { return }
        monitoringRequested = true
        guard monitor == nil else { return }
        startStandbyMonitor()
        let generation = monitorGeneration
        monitor = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                guard generation == monitorGeneration else { return }
                if !busy && !isTestingLatency && !routeMutationInFlight && running {
                    healthCheckInFlight = true
                    do {
                        try await inspectCurrent()
                        if !dynamicMode && !tun && running && !preferences.sources.isEmpty && Date().timeIntervalSince(lastUpdate) > 3600 { try await refresh() }
                    } catch is CancellationError { healthCheckInFlight = false; return }
                    catch { logBackground("检测失败：\(error.localizedDescription)") }
                    healthCheckInFlight = false
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
        if shutdownComplete { return true }
        if let shutdownTask { return await shutdownTask.value }
        // 永久关闭入口；取消退出或清理失败也不重新允许创建资源。
        terminating = true
        engine.closeGate(); proxy.closeGate()
        if persistsChanges {
            GuardianClient.shared.closeGate()
            PrivilegedClient.shared.closeGate()
        }
        startupTask?.cancel(); operationTask?.cancel(); latencyTask?.cancel()
        monitor?.cancel(); standbyMonitor?.cancel()
        rulesScheduleTask?.cancel(); rulesRefreshTask?.cancel()
        let startup = startupTask, operation = operationTask, latency = latencyTask, stoppingTask = stopTask
        let schedule = rulesScheduleTask, refresh = rulesRefreshTask
        stopping = true; busy = true; monitoringRequested = false
        let task = Task { @MainActor in
            await startup?.value
            await operation?.value
            await latency?.value
            await stoppingTask?.value
            await schedule?.value
            _ = try? await refresh?.value
            await self.pauseMonitoring()
            defer { self.stopping = false; self.busy = false }
            var failures: [String] = []
            do { try await self.stop() } catch { failures.append(error.localizedDescription) }
            if self.persistsChanges {
                do {
                    try await PrivilegedClient.shared.endSession()
                    self.engine.confirmPrivilegedSessionEnded()
                } catch { failures.append(error.localizedDescription) }
                do {
                    try await GuardianClient.shared.end()
                    self.engine.confirmGuardianEnded()
                } catch { failures.append(error.localizedDescription) }
            }
            self.aggregate.stop()
            self.subscriptionReady = false
            guard failures.isEmpty else {
                self.recovery = true
                let message = "退出恢复未完成：" + failures.joined(separator: "；") + "。已禁止重新连接，请重试停止或退出。"
                self.error = message; self.log(message)
                return false
            }
            self.recovery = false; self.systemProxy = false
            self.error = nil
            self.shutdownComplete = true
            return true
        }
        shutdownTask = task
        let result = await task.value
        shutdownTask = nil
        return result
    }
}
