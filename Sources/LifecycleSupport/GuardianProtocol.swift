import Foundation

public struct GuardianRequest: Codable, Sendable {
    public var nonce: String
    public var operation: String
    public var id: String
    public var executable: String?
    public var arguments: [String]?
    public var directory: String?
    public var environment: [String: String]?
    public init(nonce: String, operation: String, id: String = UUID().uuidString, executable: String? = nil,
                arguments: [String]? = nil, directory: String? = nil, environment: [String: String]? = nil) {
        self.nonce = nonce; self.operation = operation; self.id = id; self.executable = executable
        self.arguments = arguments; self.directory = directory; self.environment = environment
    }
}
public struct GuardianReply: Codable, Sendable {
    public var id: String
    public var error: String?
    public var pid: Int32?
    public var running: Bool?
    public var code: Int32?
    public init(id: String, error: String? = nil, pid: Int32? = nil, running: Bool? = nil, code: Int32? = nil) {
        self.id = id; self.error = error; self.pid = pid; self.running = running; self.code = code
    }
}
public struct GuardianJournal: Codable {
    public var nonce: String
    public var owner: ProcessIdentity
    public var children: [String: ProcessIdentity]
    public var phase: String
    public init(nonce: String, owner: ProcessIdentity, children: [String: ProcessIdentity] = [:], phase: String = "ready") {
        self.nonce = nonce; self.owner = owner; self.children = children; self.phase = phase
    }
}
