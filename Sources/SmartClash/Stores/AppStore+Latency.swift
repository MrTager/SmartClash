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
            guard let self else { return }
            self.latencies[id] = result
            self.testingNodeIDs.remove(id)
            if self.isTestingLatency { self.latencyCompleted += 1 }
        })
    }
    func beginLatencyTest(_ targets: [ProxyNode]? = nil) {
        guard !busy, !isTestingLatency, !recovery else { return }
        let targets = targets ?? nodes
        guard !targets.isEmpty else { error = "请先添加并更新订阅"; return }
        isTestingLatency = true; latencyCompleted = 0; latencyTotal = targets.count
        testingNodeIDs = Set(targets.map(\.id))
        latencyTask = Task { [weak self] in
            guard let self else { return }
            defer { self.isTestingLatency = false; self.testingNodeIDs.removeAll() }
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
                self.error = error.localizedDescription
            }
        }
    }
    func cancelLatencyTest() { latencyTask?.cancel() }
}
