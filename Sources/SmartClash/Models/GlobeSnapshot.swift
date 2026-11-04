import Foundation
import ProxyLogic

struct GlobeMarker: Equatable, Identifiable {
    let id: String
    let name: String
    let location: GeoLocation
    let isNode: Bool
    let active: Bool
}

struct GlobeRoute: Equatable, Identifiable {
    let id: String
    let locations: [GeoLocation]
    let selected: Bool
    let highlighted: Bool
}

struct GlobeSnapshot: Equatable {
    var markers: [GlobeMarker] = []
    var routes: [GlobeRoute] = []
    var mappedConnections = 0
    var mappedNodes = 0

    static func make(nodes: [ProxyNode], activeTrace: ChainTrace, connections: [CoreConnection], locations: [String: GeoLocation],
                     focusedConnection: String?) -> GlobeSnapshot {
        var result = GlobeSnapshot()
        var markers: [String: GlobeMarker] = [:]
        let active = Set(activeTrace.hops.map(\.id))
        for node in nodes {
            guard let server = node.payload["server"] as? String, let location = locations[server] else { continue }
            let id = "node:\(server)"
            if markers[id] == nil || active.contains(node.id) {
                markers[id] = GlobeMarker(id: id, name: node.name, location: location, isNode: true, active: active.contains(node.id))
            }
        }
        result.mappedNodes = markers.count
        let current = activeTrace.hops.compactMap { hop -> GeoLocation? in
            guard let node = nodes.first(where: { $0.id == hop.id }), let server = node.payload["server"] as? String else { return nil }
            return locations[server]
        }
        if current.count > 1 && current.count == activeTrace.hops.count && activeTrace.complete {
            result.routes.append(GlobeRoute(id: "selected-route", locations: current, selected: true, highlighted: true))
        }
        let connections = connections.sorted { ($0.id == focusedConnection ? 0 : 1) < ($1.id == focusedConnection ? 0 : 1) }
        var seen = Set<String>()
        for connection in connections {
            guard let ip = connection.metadata.geolocationKey, let destination = locations[ip] else { continue }
            let path = connection.routeNodes(in: nodes)
            let locations = path.compactMap { node -> GeoLocation? in
                guard let server = node.payload["server"] as? String else { return nil }
                return locations[server]
            }
            guard !path.isEmpty, locations.count == path.count else { continue }
            result.mappedConnections += 1
            markers["target:\(ip)"] = GlobeMarker(id: "target:\(ip)", name: connection.metadata.displayDestination,
                location: destination, isNode: false, active: connection.id == focusedConnection)
            let key = path.map(\.id).joined(separator: "/") + ":" + ip
            guard seen.insert(key).inserted, result.routes.count < 48 else { continue }
            result.routes.append(GlobeRoute(id: key, locations: locations + [destination], selected: false,
                highlighted: focusedConnection == nil || focusedConnection == connection.id))
        }
        result.markers = markers.values.sorted { $0.id < $1.id }
        return result
    }
}
