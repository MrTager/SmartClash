import Foundation
import Darwin
import ProxyLogic

actor GeoIPService {
    private struct Entry: Codable {
        let location: GeoLocation
        let date: Date
    }
    private struct Response: Decodable {
        let success: Bool
        let latitude: Double?
        let longitude: Double?
        let country: String?
        let city: String?
    }
    private var cache: [String: Entry] = [:]
    private let session: URLSession
    private let cacheURL: URL?

    init(cacheURL: URL? = Files.root.appendingPathComponent("geoip-cache.json")) {
        self.cacheURL = cacheURL
        if let cacheURL, let data = try? Data(contentsOf: cacheURL),
           let saved = try? JSONDecoder().decode([String: Entry].self, from: data) {
            cache = saved.filter { Date().timeIntervalSince($0.value.date) < 86_400 && $0.value.location.isValid }
        }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.connectionProxyDictionary = [:]
        configuration.timeoutIntervalForRequest = 5
        configuration.timeoutIntervalForResource = 6
        session = URLSession(configuration: configuration)
    }

    func location(for host: String) async -> GeoLocation? {
        guard !Task.isCancelled, let ip = Self.publicAddress(for: host) else { return nil }
        if let entry = cache[ip], Date().timeIntervalSince(entry.date) < 86_400 { return entry.location }
        // Only a validated public IP is sent. Never send node names, credentials, or requested hostnames.
        guard let url = URL(string: "https://ipwho.is/\(ip)") else { return nil }
        do {
            let (data, response) = try await session.data(from: url)
            guard !Task.isCancelled, let http = response as? HTTPURLResponse, http.statusCode == 200,
                  data.count < 65_536 else { return nil }
            let result = try JSONDecoder().decode(Response.self, from: data)
            guard result.success, let lat = result.latitude, let lon = result.longitude, let country = result.country else { return nil }
            let location = GeoLocation(latitude: lat, longitude: lon, country: country, city: result.city ?? "")
            guard location.isValid else { return nil }
            cache[ip] = Entry(location: location, date: Date())
            if cache.count > 2000 { cache = cache.sorted { $0.value.date > $1.value.date }.prefix(1000).reduce(into: [:]) { $0[$1.key] = $1.value } }
            if let cacheURL, let encoded = try? JSONEncoder().encode(cache) { try? Files.write(encoded, to: cacheURL) }
            return location
        } catch { return nil }
    }

    nonisolated static func publicAddress(for host: String) -> String? {
        if PublicIPAddress.isPublic(host) { return host }
        var address = in6_addr()
        if inet_pton(AF_INET6, host, &address) == 1 || host.split(separator: ".").allSatisfy({ UInt8($0) != nil }) { return nil }
        guard !host.isEmpty, host.count <= 253, !host.hasSuffix(".local"), host != "localhost",
              host.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "." || $0 == "-") }) else { return nil }
        var hints = addrinfo(); hints.ai_family = AF_UNSPEC; hints.ai_socktype = SOCK_STREAM
        var results: UnsafeMutablePointer<addrinfo>?
        guard getaddrinfo(host, nil, &hints, &results) == 0, let head = results else { return nil }
        defer { freeaddrinfo(head) }
        var cursor: UnsafeMutablePointer<addrinfo>? = head
        var addresses: [String] = []
        while let current = cursor {
            var buffer = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(current.pointee.ai_addr, current.pointee.ai_addrlen, &buffer, socklen_t(buffer.count), nil, 0, NI_NUMERICHOST) == 0 {
                let ip = String(cString: buffer)
                if PublicIPAddress.isPublic(ip) { addresses.append(ip) }
            }
            cursor = current.pointee.ai_next
        }
        return addresses.first(where: { !$0.contains(":") }) ?? addresses.first
    }
}
