import Foundation
import Combine

struct SavedNode: Identifiable, Codable {
    var id = UUID()
    var name: String
    var host: String
    var port: UInt16 = 7337
    var authKey: String = ""
    var lastConnected: Date?
}

enum HistoryStatus: String, Codable {
    case connected = "Connected"
    case disconnected = "Disconnected"
    case timedOut = "Timed Out"
}

struct ConnectionHistoryEntry: Identifiable, Codable {
    var id = UUID()
    var nodeName: String
    var host: String
    var startTime: Date
    var endTime: Date?
    var status: HistoryStatus = .connected

    var durationString: String {
        guard let endTime else { return "Active" }
        let d = Int(endTime.timeIntervalSince(startTime))
        let h = d / 3600
        let m = (d % 3600) / 60
        let s = d % 60
        return String(format: "%02d:%02d:%02d", h, m, s)
    }

    private static let relativeFormatter: RelativeDateTimeFormatter = {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .abbreviated
        return f
    }()

    var relativeTimeString: String {
        Self.relativeFormatter.localizedString(for: startTime, relativeTo: Date())
    }
}

@MainActor
final class ConnectionStore: ObservableObject {
    @Published var savedNodes: [SavedNode] = []
    @Published var history: [ConnectionHistoryEntry] = []

    private let nodesKey = "boo.remote.savedNodes"
    private let historyKey = "boo.remote.connectionHistory"
    private let maxHistory = 50

    init() {
        loadNodes()
        loadHistory()
    }

    func addNode(_ node: SavedNode) {
        savedNodes.append(node)
        saveNodes()
    }

    func updateNode(_ node: SavedNode) {
        guard let index = savedNodes.firstIndex(where: { $0.id == node.id }) else { return }
        savedNodes[index] = node
        saveNodes()
    }

    func updateNodeLastConnected(_ nodeId: UUID) {
        guard let index = savedNodes.firstIndex(where: { $0.id == nodeId }) else { return }
        savedNodes[index].lastConnected = Date()
        saveNodes()
    }

    func recordConnection(nodeName: String, host: String) -> UUID {
        let entry = ConnectionHistoryEntry(nodeName: nodeName, host: host, startTime: Date())
        history.insert(entry, at: 0)
        history = Array(history.prefix(maxHistory))
        saveHistory()
        return entry.id
    }

    func endConnection(id: UUID, status: HistoryStatus) {
        guard let index = history.firstIndex(where: { $0.id == id }) else { return }
        history[index].endTime = Date()
        history[index].status = status
        saveHistory()
    }

    func clearHistory() {
        history.removeAll()
        saveHistory()
    }

    private func loadNodes() {
        guard let data = UserDefaults.standard.data(forKey: nodesKey),
              let nodes = try? JSONDecoder().decode([SavedNode].self, from: data) else { return }
        savedNodes = nodes
    }

    private func saveNodes() {
        guard let data = try? JSONEncoder().encode(savedNodes) else { return }
        UserDefaults.standard.set(data, forKey: nodesKey)
    }

    private func loadHistory() {
        guard let data = UserDefaults.standard.data(forKey: historyKey),
              let entries = try? JSONDecoder().decode([ConnectionHistoryEntry].self, from: data) else { return }
        history = entries
    }

    private func saveHistory() {
        guard let data = try? JSONEncoder().encode(history) else { return }
        UserDefaults.standard.set(data, forKey: historyKey)
    }
}

enum ConnectionStatus: Equatable {
    case disconnected
    case connecting
    case connected
    case authenticated
    case attached(sessionId: UInt32)
    case connectionLost(reason: String)
}

@MainActor
final class ConnectionMonitor: ObservableObject {
    @Published var status: ConnectionStatus = .disconnected

    private let client: GSPClient
    private var cancellables = Set<AnyCancellable>()

    private(set) var lastHost: String?
    private(set) var lastPort: UInt16?
    private(set) var lastAuthKey: String?
    private(set) var currentHistoryId: UUID?
    private(set) var currentNodeId: UUID?

    init(client: GSPClient) {
        self.client = client
        observe()
    }

    private func observe() {
        Publishers.CombineLatest4(
            client.$connected,
            client.$authenticated,
            client.$attachedSessionId,
            client.$lastError
        )
        .receive(on: DispatchQueue.main)
        .sink { [weak self] connected, authenticated, sessionId, error in
            guard let self else { return }

            if let sessionId {
                self.status = .attached(sessionId: sessionId)
            } else if let error, !connected, self.status != .disconnected {
                self.status = .connectionLost(reason: error)
            } else if authenticated {
                self.status = .authenticated
            } else if connected {
                self.status = .connected
            } else if self.lastHost != nil, case .attached = self.status {
                self.status = .connectionLost(reason: "Connection closed")
            } else {
                self.status = .disconnected
            }
        }
        .store(in: &cancellables)
    }

    func connect(host: String, port: UInt16, authKey: String = "", historyId: UUID? = nil, nodeId: UUID? = nil) {
        lastHost = host
        lastPort = port
        lastAuthKey = authKey
        currentHistoryId = historyId
        currentNodeId = nodeId
        status = .connecting
        client.connect(host: host, port: port, authKey: authKey)
    }

    func reconnect() {
        guard let host = lastHost, let port = lastPort else { return }
        connect(host: host, port: port, authKey: lastAuthKey ?? "")
    }

    func disconnect() {
        client.disconnect()
        status = .disconnected
        lastHost = nil
        lastPort = nil
        lastAuthKey = nil
        currentHistoryId = nil
        currentNodeId = nil
    }

    func clearTrackedConnection() {
        currentHistoryId = nil
        currentNodeId = nil
    }
}
