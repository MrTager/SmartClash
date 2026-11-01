import Foundation
import Network
import ProxyLogic
final class AggregateServer {
    private let queue = DispatchQueue(label: "com.smartclash.subscription")
    private var listener: NWListener?
    private var payload = Data()
    private let token: String
    init(token: String) { self.token = token }
    var address: String { "http://127.0.0.1:17892/\(token)/config.yaml" }
    func update(_ nodes: [ProxyNode]) throws {
        let data = try Configuration.export(nodes: nodes)
        queue.sync { payload = data }
    }
    func start(onError: @escaping (String) -> Void) throws {
        let parameters = NWParameters.tcp
        parameters.requiredLocalEndpoint = .hostPort(host: NWEndpoint.Host("127.0.0.1"), port: 17892)
        let listener = try NWListener(using: parameters)
        listener.stateUpdateHandler = { state in
            if case .failed(_) = state { onError("本机订阅端口 17892 无法监听，请检查端口冲突") }
        }
        listener.newConnectionHandler = { [weak self] connection in
            guard let self else { connection.cancel(); return }
            connection.start(queue: self.queue)
            // A stalled local connection cannot retain resources indefinitely.
            self.queue.asyncAfter(deadline: .now() + 5) { connection.cancel() }
            self.receive(connection, buffer: Data())
        }
        self.listener = listener; listener.start(queue: queue)
    }
    private func receive(_ connection: NWConnection, buffer: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 8192) { [weak self] data, _, complete, error in
            guard let self else { connection.cancel(); return }
            var request = buffer; request.append(data ?? Data())
            guard request.count <= 8192 else { connection.cancel(); return }
            if let text = String(data: request, encoding: .utf8), text.contains("\r\n\r\n") {
                let valid = text.components(separatedBy: "\r\n").first == "GET /\(self.token)/config.yaml HTTP/1.1" || text.components(separatedBy: "\r\n").first == "GET /\(self.token)/config.yaml HTTP/1.0"
                let body = valid ? self.payload : Data("Not found".utf8)
                let header = "HTTP/1.1 \(valid ? "200 OK" : "404 Not Found")\r\nContent-Type: application/yaml; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: \(body.count)\r\nConnection: close\r\n\r\n"
                var response = Data(header.utf8); response.append(body)
                connection.send(content: response, completion: .contentProcessed { _ in connection.cancel() })
            } else if complete || error != nil { connection.cancel() }
            else { self.receive(connection, buffer: request) }
        }
    }
}
