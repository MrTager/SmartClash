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
    @Published private(set) var expectingData = false
    @Published private(set) var usingFallback = false
    @Published private(set) var updatedAt: Date?
    @Published private(set) var points: [Point] = []
    private var ledger = TrafficLedger()
    private var task: Task<Void, Never>?
    private var lastSave = Date.distantPast
    private let filename: URL?
    private var generation = UUID()
    private var lastStreamSample: Date?
    private var previousSnapshot: (up: UInt64, down: UInt64, time: Date)?
    init(filename: URL? = Files.root.appendingPathComponent("traffic-history.json")) {
        self.filename = filename
        if let filename, let data = try? Data(contentsOf: filename), let saved = try? JSONDecoder().decode(TrafficLedger.self, from: data) {
            ledger = saved; lifetimeUpload = saved.lifetimeUpload; lifetimeDownload = saved.lifetimeDownload
        }
    }
    func start(client: CoreClient, runID: String) {
        task?.cancel(); sessionUpload = 0; sessionDownload = 0; points = []
        connected = false; expectingData = true; usingFallback = false; updatedAt = nil
        uploadRate = 0; downloadRate = 0; lastStreamSample = nil; previousSnapshot = nil
        let token = UUID(); generation = token
        task = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled && generation == token {
                do {
                    for try await sample in client.trafficStream() {
                        guard !Task.isCancelled, generation == token else { return }
                        ingest(sample, runID: runID)
                    }
                } catch { if Task.isCancelled { return } }
                guard generation == token else { return }
                if Date().timeIntervalSince(updatedAt ?? .distantPast) > 3 {
                    connected = false; uploadRate = 0; downloadRate = 0
                }
                do { try await Task.sleep(nanoseconds: 2_000_000_000) } catch { return }
            }
        }
    }
    func stop() {
        generation = UUID(); task?.cancel(); task = nil; save()
        connected = false; expectingData = false; uploadRate = 0; downloadRate = 0
        lastStreamSample = nil; previousSnapshot = nil
    }
    func ingest(_ sample: TrafficSample, runID: String, at time: Date = Date()) {
        lastStreamSample = time; usingFallback = false
        recordPoint(upload: sample.up, download: sample.down, at: time)
        if let up = sample.upTotal, let down = sample.downTotal {
            recordTotals(upload: up, download: down, runID: runID)
        }
    }
    func ingestSnapshot(_ snapshot: ConnectionSnapshot, runID: String, at time: Date = Date()) {
        recordTotals(upload: snapshot.uploadTotal, download: snapshot.downloadTotal, runID: runID)
        defer { previousSnapshot = (snapshot.uploadTotal, snapshot.downloadTotal, time) }
        guard time.timeIntervalSince(lastStreamSample ?? .distantPast) > 3 else { return }
        guard let previousSnapshot else { return }
        let seconds = time.timeIntervalSince(previousSnapshot.time)
        guard seconds > 0, seconds < 10 else { return }
        let up = snapshot.uploadTotal >= previousSnapshot.up ? snapshot.uploadTotal - previousSnapshot.up : 0
        let down = snapshot.downloadTotal >= previousSnapshot.down ? snapshot.downloadTotal - previousSnapshot.down : 0
        usingFallback = true
        recordPoint(upload: UInt64(Double(up) / seconds), download: UInt64(Double(down) / seconds), at: time)
    }
    private func recordPoint(upload: UInt64, download: UInt64, at time: Date) {
        connected = true; updatedAt = time; uploadRate = upload; downloadRate = download
        points.append(Point(time: time, upload: upload, download: download))
        points.removeAll { time.timeIntervalSince($0.time) > 60 }
        if points.count > 60 { points.removeFirst(points.count - 60) }
    }
    private func recordTotals(upload: UInt64, download: UInt64, runID: String) {
        // Ignore older totals if a stream sample and a snapshot cross in flight.
        if ledger.runID == runID && (upload < ledger.lastUpload || download < ledger.lastDownload) { return }
        sessionUpload = upload; sessionDownload = download
        ledger.record(upload: upload, download: download, runID: runID)
        lifetimeUpload = ledger.lifetimeUpload; lifetimeDownload = ledger.lifetimeDownload
        if Date().timeIntervalSince(lastSave) >= 5 { save() }
    }
    func markUnavailable(at time: Date = Date()) {
        if time.timeIntervalSince(updatedAt ?? .distantPast) > 3 {
            connected = false; uploadRate = 0; downloadRate = 0
        }
    }
    var status: String {
        if connected { return usingFallback ? "实时 · 累计差值恢复" : "实时 · 每秒更新" }
        return expectingData ? "正在重连流量统计…" : "连接后显示实时流量"
    }
    private func save() {
        guard let filename else { return }
        if let data = try? JSONEncoder().encode(ledger) { try? Files.write(data, to: filename); lastSave = Date() }
    }
    static func bytes(_ value: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: value), countStyle: .binary)
    }
}
