import Foundation
import ProxyLogic

extension AppStore {
    var nodeListNodes: [ProxyNode] {
        let ids = Set(nodes.map(\.id))
        return nodes + routeNodes.filter { !ids.contains($0.id) }
    }
    func saveLatencies() {
        guard persistsChanges else { return }
        do { try Files.write(JSONEncoder().encode(latencies), to: Files.root.appendingPathComponent("latencies.json")) }
        catch { self.error = "无法保存本机延迟测试结果" }
    }
    func measureLatencies(_ targets: [ProxyNode]) async {
        let client = api!
        await LatencyTester.run(targets.map { LatencyRequest(id: $0.id, name: $0.name) }, probe: { request in
            await client.delay(request.name)
        }, receive: { [weak self] id, result in
            guard let self, !Task.isCancelled else { return }
            self.latencies[id] = result
            if self.isTestingLatency { self.latencyCompleted += 1 }
        })
    }
    func beginLatencyTest(_ targets: [ProxyNode]? = nil) {
        guard !recovery else { return }
        let targets = targets ?? nodes
        guard !targets.isEmpty else { error = "请先添加并更新订阅"; return }
        latencyTask = perform { [weak self] in
            guard let self else { return }
            self.isTestingLatency = true; self.latencyCompleted = 0; self.latencyTotal = targets.count
            defer { self.isTestingLatency = false }
            do {
                if !self.running {
                    try await self.start(purpose: .latencyTest)
                    self.internetStatus = "仅测速内核 · 系统代理 / TUN 未开启"
                }
                try Task.checkCancellation()
                await self.measureLatencies(targets)
                self.saveLatencies()
                self.log(Task.isCancelled ? "延迟测试已取消，已保留完成的结果" : "延迟测试完成：\(self.latencyCompleted)/\(self.latencyTotal)，业务节点未改变")
            } catch is CancellationError {
                self.log("延迟测试已取消")
            } catch {
                if !Task.isCancelled { self.error = error.localizedDescription }
            }
        }
    }
    func cancelLatencyTest() { latencyTask?.cancel() }
}
