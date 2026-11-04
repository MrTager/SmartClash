import Foundation
import Combine
import ProxyLogic

@MainActor
final class TrafficStore: ObservableObject {
    struct Point: Identifiable {
        let id = UUID()
        let time: Date
        let upload: UInt64
        let download: UInt64
    }
    @Published private(set) var uploadRate: UInt64 = 0
    @Published private(set) var downloadRate: UInt64 = 0
    @Published private(set) var sessionUpload: UInt64 = 0
    @Published private(set) var sessionDownload: UInt64 = 0
    @Published private(set) var lifetimeUpload: UInt64 = 0
    @Published private(set) var lifetimeDownload: UInt64 = 0
    @Published private(set) var connected = false
    @Published private(set) var points: [Point] = []
    private var ledger = TrafficLedger()
    private var task: Task<Void, Never>?
    private var lastSave = Date.distantPast
    private let filename: URL?
    init(filename: URL? = Files.root.appendingPathComponent("traffic-history.json")) {
        self.filename = filename
        if let filename, let data = try? Data(contentsOf: filename), let saved = try? JSONDecoder().decode(TrafficLedger.self, from: data) {
            ledger = saved; lifetimeUpload = saved.lifetimeUpload; lifetimeDownload = saved.lifetimeDownload
        }
    }
    func start(client: CoreClient, runID: String) {
        task?.cancel(); sessionUpload = 0; sessionDownload = 0; points = []
        task = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                do {
                    for try await sample in client.trafficStream() {
                        guard !Task.isCancelled else { return }
                        ingest(sample, runID: runID)
                    }
                } catch { if Task.isCancelled { return } }
                connected = false; uploadRate = 0; downloadRate = 0
                do { try await Task.sleep(nanoseconds: 2_000_000_000) } catch { return }
            }
        }
    }
    func stop() {
        task?.cancel(); task = nil; save()
        connected = false; uploadRate = 0; downloadRate = 0
    }
    func ingest(_ sample: TrafficSample, runID: String, at time: Date = Date()) {
        connected = true; uploadRate = sample.up; downloadRate = sample.down
        points.append(Point(time: time, upload: sample.up, download: sample.down))
        if points.count > 60 { points.removeFirst(points.count - 60) }
        if let up = sample.upTotal, let down = sample.downTotal {
            sessionUpload = up; sessionDownload = down
            ledger.record(upload: up, download: down, runID: runID)
            lifetimeUpload = ledger.lifetimeUpload; lifetimeDownload = ledger.lifetimeDownload
            if Date().timeIntervalSince(lastSave) >= 5 { save() }
        }
    }
    private func save() {
        guard let filename else { return }
        if let data = try? JSONEncoder().encode(ledger) { try? Files.write(data, to: filename); lastSave = Date() }
    }
    static func bytes(_ value: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: value), countStyle: .binary)
    }
}
