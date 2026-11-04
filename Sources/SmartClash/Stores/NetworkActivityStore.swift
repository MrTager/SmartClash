import Foundation
import Combine
import ProxyLogic

@MainActor
final class NetworkActivityStore: ObservableObject {
    @Published private(set) var connections: [CoreConnection] = []
    @Published private(set) var locations: [String: GeoLocation] = [:]
    @Published private(set) var connected = false
    @Published private(set) var enabled = false
    @Published private(set) var updatedAt: Date?
    @Published private(set) var resolving = false
    private var nodes: [ProxyNode] = []
    private var preferredServers: [String] = []
    private var pollTask: Task<Void, Never>?
    private var geoTask: Task<Void, Never>?
    private var attempted: [String: Date] = [:]
    private var generation = UUID()
    private let geo: GeoIPService

    init(geo: GeoIPService = GeoIPService()) { self.geo = geo }

    func configure(nodes: [ProxyNode], active: [ProxyNode]) {
        self.nodes = nodes
        preferredServers = active.compactMap { $0.payload["server"] as? String }
    }

    func start(client: CoreClient, traffic: TrafficStore, runID: String) {
        stop(); enabled = true; let token = UUID(); generation = token
        pollTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled && generation == token {
                do {
                    let snapshot = try await client.connections()
                    guard !Task.isCancelled, generation == token else { return }
                    ingest(snapshot)
                    traffic.ingestSnapshot(snapshot, runID: runID)
                } catch {
                    guard !Task.isCancelled, generation == token else { return }
                    connected = false; connections = []
                    traffic.markUnavailable()
                }
                do { try await Task.sleep(nanoseconds: 1_000_000_000) } catch { return }
            }
        }
        geoTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled && generation == token {
                let candidates = preferredServers + connections.prefix(80).flatMap { connection in
                    connection.routeNodes(in: self.nodes).compactMap { $0.payload["server"] as? String } +
                    [connection.metadata.geolocationKey].compactMap { $0 }
                } + nodes.compactMap { $0.payload["server"] as? String }
                if let host = candidates.first(where: { self.locations[$0] == nil && Date().timeIntervalSince(self.attempted[$0] ?? .distantPast) > 300 }) {
                    attempted[host] = Date(); resolving = true
                    let location = await geo.location(for: host)
                    guard !Task.isCancelled, generation == token else { return }
                    if let location { locations[host] = location }
                    resolving = false
                }
                do { try await Task.sleep(nanoseconds: 1_200_000_000) } catch { return }
            }
        }
    }

    func ingest(_ snapshot: ConnectionSnapshot, at date: Date = Date()) {
        connections = snapshot.connections.sorted {
            let left = $0.download.addingReportingOverflow($0.upload).partialValue
            let right = $1.download.addingReportingOverflow($1.upload).partialValue
            return left == right ? $0.id < $1.id : left > right
        }
        connected = true; updatedAt = date
    }

    func stop() {
        generation = UUID(); pollTask?.cancel(); geoTask?.cancel(); pollTask = nil; geoTask = nil
        connected = false; enabled = false; resolving = false; connections = []; updatedAt = nil
    }
}
