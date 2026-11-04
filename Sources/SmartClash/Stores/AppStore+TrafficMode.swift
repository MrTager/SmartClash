import Foundation
import ProxyLogic

extension AppStore {
    func changeTrafficMode(_ wanted: TrafficMode) async throws {
        guard wanted != trafficMode else { return }
        guard !recovery else { throw ProxyError.message("请先恢复上次网络设置") }
        let previous = trafficMode
        if running {
            if wanted != .direct && nodes.isEmpty { throw ProxyError.message("请先停止内核并添加订阅，才能使用代理模式") }
            if wanted == .rule { try await ensureRulesReady() }
            let previousStatus = routingStatus
            let previousActiveMode = activeMode
            let previousInternetStatus = internetStatus
            if previous == .direct && wanted != .direct {
                do { try await ensureReachableRoute() }
                catch {
                    routingStatus = previousStatus; activeMode = previousActiveMode; internetStatus = previousInternetStatus
                    throw error
                }
            }
            do { try await api.setTrafficMode(wanted) }
            catch {
                // Finish rollback even if a second click cancelled the first operation.
                let restored = await Task { @MainActor in
                    do { try await api.setTrafficMode(previous); return true }
                    catch { return false }
                }.value
                if !restored { recovery = true; log("流量模式回滚未确认，请停止并恢复网络") }
                routingStatus = previousStatus
                activeMode = previousActiveMode; internetStatus = previousInternetStatus
                throw error
            }
        }
        preferences.trafficMode = wanted; savePreferences()
        if wanted == .direct {
            activeMode = nil; routingStatus = running ? "直连模式" : "尚未连接"
            internetStatus = running ? "直连模式已开启" : "尚未连接"
            openAIStatus = "直连模式不检测代理节点"
        } else if running { routingStatus = "已打通"; openAIStatus = "OpenAI 尚未检测" }
        publishRoute()
        log("流量模式：\(wanted.title)")
        if running {
            // Existing TCP/UDP sessions otherwise retain the route of the old mode.
            do { try await api.closeConnections() }
            catch { log("模式已切换，旧连接关闭未完成；已有请求可能需要重试") }
        }
    }

    func startRuleSchedule() {
        guard persistsChanges, rulesScheduleTask == nil else { return }
        rulesScheduleTask = Task { [weak self] in
            var retryAfter = Date.distantPast
            while !Task.isCancelled {
                guard let self else { return }
                if Date() >= retryAfter && (self.ruleSnapshot?.isDue() ?? true) {
                    do { try await self.refreshRules(); retryAfter = .distantPast }
                    catch is CancellationError { return }
                    catch {
                        self.ruleUpdateMessage = "更新失败，\(self.ruleSnapshot == nil ? "尚无可用规则" : "保留旧规则")；5 分钟后重试"
                        self.logBackground("规则更新失败：\(error.localizedDescription)")
                        retryAfter = Date().addingTimeInterval(300)
                    }
                } else if self.running && !self.rulesAppliedToCore && self.ruleSnapshot != nil && !self.busy {
                    do { try await self.api.updateRuleProviders(); self.rulesAppliedToCore = true }
                    catch { self.logBackground("规则内核应用失败，稍后重试") }
                }
                do { try await Task.sleep(nanoseconds: 60_000_000_000) } catch { return }
            }
        }
    }

    func ensureRulesReady() async throws {
        if ruleSnapshot == nil { try await refreshRules() }
        guard ruleSnapshot?.isComplete == true else { throw ProxyError.message("尚无完整规则，请先更新规则，或使用全局 / 直连模式") }
        if running && !rulesAppliedToCore && ruleProviderBaseURL != nil {
            try await api.updateRuleProviders(); rulesAppliedToCore = true
        }
    }

    func refreshRules() async throws {
        if rulesUpdating {
            while rulesUpdating { try await Task.sleep(nanoseconds: 100_000_000) }
            guard ruleSnapshot != nil else { throw ProxyError.message("规则更新未完成，请稍后重试") }
            return
        }
        guard persistsChanges else { throw ProxyError.message("离线验收实例不下载规则") }
        rulesUpdating = true; ruleUpdateMessage = "正在下载并转换规则…"
        defer { rulesUpdating = false; rulesRefreshTask = nil }
        let throughProxy = running && trafficMode != .direct
        let task = Task.detached(priority: .utility) { try await RuleSubscriptionService.fetch(throughProxy: throughProxy) }
        rulesRefreshTask = task
        do {
            let snapshot = try await withTaskCancellationHandler { try await task.value } onCancel: { task.cancel() }
            try Task.checkCancellation()
            // Only publish once all three modules have downloaded and passed conversion.
            let data = try await Task.detached(priority: .utility) { try JSONEncoder().encode(snapshot) }.value
            try Task.checkCancellation()
            try Files.write(data, to: Files.root.appendingPathComponent("routing-rules.json"))
            try aggregate.updateRules(snapshot)
            ruleSnapshot = snapshot
            rulesAppliedToCore = false
            if running {
                try await api.updateRuleProviders(); rulesAppliedToCore = true
            }
            ruleUpdateMessage = "已更新 \(snapshot.count) 条规则"
            log("规则订阅更新完成：\(snapshot.count) 条生效，\(snapshot.skipped) 条不兼容规则已跳过；每小时自动更新")
        } catch {
            ruleUpdateMessage = ruleSnapshot == nil ? "规则更新失败，尚无可用规则" : "规则更新失败，保留已下载规则"
            throw error
        }
    }
}
