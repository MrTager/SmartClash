import Foundation
import ProxyLogic

struct LatencyRequest: Sendable {
    let id: String
    let name: String
}

enum LatencyTester {
    static func run(_ nodes: [LatencyRequest], concurrency: Int = 6,
                    probe: @escaping @Sendable (LatencyRequest) async -> NodeLatency,
                    receive: @escaping @MainActor (String, NodeLatency) -> Void) async {
        guard !Task.isCancelled else { return }
        await withTaskGroup(of: (String, NodeLatency).self) { group in
            var iterator = nodes.makeIterator()
            for _ in 0..<min(max(1, concurrency), nodes.count) {
                if let node = iterator.next() { group.addTask { (node.id, await probe(node)) } }
            }
            for await (id, result) in group {
                guard !Task.isCancelled else { group.cancelAll(); break }
                await receive(id, result)
                if let node = iterator.next() { group.addTask { (node.id, await probe(node)) } }
            }
        }
    }
}
