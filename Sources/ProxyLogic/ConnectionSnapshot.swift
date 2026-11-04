import Foundation

public struct ConnectionMetadata: Codable, Equatable, Sendable {
    public var network: String?
    public var host: String?
    public var destinationIP: String?
    public var remoteDestination: String?
    public var destinationPort: String?
    public var process: String?

    public init(network: String? = nil, host: String? = nil, destinationIP: String? = nil,
                destinationPort: String? = nil, process: String? = nil, remoteDestination: String? = nil) {
        self.network = network; self.host = host; self.destinationIP = destinationIP
        self.destinationPort = destinationPort; self.process = process
        self.remoteDestination = remoteDestination
    }

    public var displayDestination: String {
        if let host, !host.isEmpty { return host }
        if let destinationIP, !destinationIP.isEmpty { return destinationIP }
        return "未知目标"
    }
    public var publicDestinationIP: String? {
        destinationIP.flatMap { PublicIPAddress.isPublic($0) ? $0 : nil }
    }
    public var geolocationKey: String? {
        if let publicDestinationIP { return publicDestinationIP }
        // remoteDestination is the proxy peer, NOT the website's destination IP.
        guard let host, !host.isEmpty else { return nil }
        return host.lowercased()
    }
}

public struct CoreConnection: Codable, Identifiable, Equatable, Sendable {
    public let id: String
    public let metadata: ConnectionMetadata
    public let upload: UInt64
    public let download: UInt64
    public let chains: [String]
    public let rule: String?

    public init(id: String, metadata: ConnectionMetadata, upload: UInt64 = 0, download: UInt64 = 0,
                chains: [String] = [], rule: String? = nil) {
        self.id = id; self.metadata = metadata; self.upload = upload; self.download = download
        self.chains = chains; self.rule = rule
    }

    public var pathLabel: String {
        if chains.contains("DIRECT") { return "直连" }
        if chains.contains("REJECT") || chains.contains("REJECT-DROP") { return "已拦截" }
        return chains.filter { !["SMART", "PROBE", "GLOBAL"].contains($0) }.reversed().joined(separator: " → ")
    }

    /// The connection's own chain preserves old connections after the SMART group switches.
    /// Never infer a connection's route from the group's current selection.
    public func routeNodes(in nodes: [ProxyNode]) -> [ProxyNode] {
        guard !chains.contains("DIRECT"), !chains.contains("REJECT"), !chains.contains("REJECT-DROP") else { return [] }
        let index = Dictionary(nodes.map { ($0.name, $0) }, uniquingKeysWith: { _, rhs in rhs })
        guard let exit = chains.first(where: { index[$0] != nil }) else { return [] }
        var result: [ProxyNode] = []; var visited = Set<String>(); var cursor: String? = exit
        while let name = cursor {
            guard let node = index[name], visited.insert(name).inserted, result.count < 16 else { return [] }
            result.append(node); cursor = node.payload["dialer-proxy"] as? String
        }
        return result.reversed()
    }
}

public struct ConnectionSnapshot: Decodable, Sendable {
    public let uploadTotal: UInt64
    public let downloadTotal: UInt64
    public let connections: [CoreConnection]

    public init(uploadTotal: UInt64, downloadTotal: UInt64, connections: [CoreConnection]) {
        self.uploadTotal = uploadTotal; self.downloadTotal = downloadTotal; self.connections = connections
    }

    private enum CodingKeys: String, CodingKey { case uploadTotal, downloadTotal, connections }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        uploadTotal = try values.decode(UInt64.self, forKey: .uploadTotal)
        downloadTotal = try values.decode(UInt64.self, forKey: .downloadTotal)
        connections = try values.decodeIfPresent([CoreConnection].self, forKey: .connections) ?? []
    }
}

public struct GeoLocation: Codable, Equatable, Hashable, Sendable {
    public let latitude: Double
    public let longitude: Double
    public let country: String
    public let city: String
    public init(latitude: Double, longitude: Double, country: String, city: String = "") {
        self.latitude = latitude; self.longitude = longitude; self.country = country; self.city = city
    }
    public var label: String { city.isEmpty ? country : "\(city), \(country)" }
    public var isValid: Bool {
        latitude.isFinite && longitude.isFinite && (-90...90).contains(latitude) && (-180...180).contains(longitude)
    }
}
