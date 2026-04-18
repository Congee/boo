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
    private let trustedInstancesKey = "boo.remote.trustedServerInstances"
    private let maxHistory = 50
    private var trustedServerInstances: [String: String] = [:]

    init() {
        loadNodes()
        loadHistory()
        loadTrustedServerInstances()
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

    func recordTrustedServerInstance(host: String, port: UInt16, instanceId: String) -> String? {
        let key = "\(host):\(port)"
        if let existing = trustedServerInstances[key] {
            guard existing == instanceId else {
                return "Server identity changed for \(key). Expected \(existing), got \(instanceId)."
            }
            return nil
        }
        trustedServerInstances[key] = instanceId
        saveTrustedServerInstances()
        return nil
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

    private func loadTrustedServerInstances() {
        guard let data = UserDefaults.standard.data(forKey: trustedInstancesKey),
              let instances = try? JSONDecoder().decode([String: String].self, from: data) else { return }
        trustedServerInstances = instances
    }

    private func saveTrustedServerInstances() {
        guard let data = try? JSONEncoder().encode(trustedServerInstances) else { return }
        UserDefaults.standard.set(data, forKey: trustedInstancesKey)
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

enum TransportHealth: Equatable {
    case idle
    case healthy
    case degraded(reason: String)
    case lost(reason: String)
}

enum ReconnectState: Equatable {
    case idle
    case waiting(attempt: Int, nextRetryIn: TimeInterval)
    case failed(reason: String)
}

@MainActor
final class ConnectionMonitor: ObservableObject {
    @Published var status: ConnectionStatus = .disconnected
    @Published var transportHealth: TransportHealth = .idle
    @Published var reconnectState: ReconnectState = .idle

    private let client: GSPClient
    private var cancellables = Set<AnyCancellable>()
    private var heartbeatTimer: AnyCancellable?
    private var reconnectWorkItem: DispatchWorkItem?

    private static let degradedHeartbeatAge: TimeInterval = 8
    private static let lostHeartbeatAge: TimeInterval = 15
    private static let reconnectDelay: TimeInterval = 2
    private static let maxReconnectAttempts = 5

    private var reconnectAllowed = false
    private var reconnectAttempt = 0

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
        Publishers.CombineLatest(
            Publishers.CombineLatest4(
                client.$connected,
                client.$authenticated,
                client.$attachedSessionId,
                client.$lastError
            ),
            client.$lastHeartbeatAck
        )
        .receive(on: DispatchQueue.main)
        .sink { [weak self] values, lastHeartbeatAck in
            guard let self else { return }
            let (connected, authenticated, sessionId, error) = values

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

            self.updateTransportHealth(
                connected: connected,
                authenticated: authenticated,
                error: error,
                lastHeartbeatAck: lastHeartbeatAck
            )
            self.updateReconnectState(
                connected: connected,
                authenticated: authenticated,
                sessionId: sessionId,
                error: error
            )
        }
        .store(in: &cancellables)

        heartbeatTimer = Timer
            .publish(every: 1, on: .main, in: .common)
            .autoconnect()
            .sink { [weak self] _ in
                guard let self else { return }
                self.updateTransportHealth(
                    connected: self.client.connected,
                    authenticated: self.client.authenticated,
                    error: self.client.lastError,
                    lastHeartbeatAck: self.client.lastHeartbeatAck
                )
            }
    }

    func connect(host: String, port: UInt16, authKey: String = "", historyId: UUID? = nil, nodeId: UUID? = nil) {
        lastHost = host
        lastPort = port
        lastAuthKey = authKey
        currentHistoryId = historyId
        currentNodeId = nodeId
        status = .connecting
        transportHealth = .idle
        reconnectAllowed = true
        cancelReconnect()
        client.connect(host: host, port: port, authKey: authKey)
    }

    func reconnect() {
        guard let host = lastHost, let port = lastPort else { return }
        connect(host: host, port: port, authKey: lastAuthKey ?? "")
    }

    func disconnect() {
        reconnectAllowed = false
        cancelReconnect()
        client.disconnect()
        status = .disconnected
        transportHealth = .idle
        reconnectState = .idle
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

    private func updateTransportHealth(
        connected: Bool,
        authenticated: Bool,
        error: String?,
        lastHeartbeatAck: Date?
    ) {
        if let error, !connected {
            transportHealth = .lost(reason: error)
            return
        }
        guard connected else {
            transportHealth = .idle
            return
        }
        guard authenticated else {
            transportHealth = .healthy
            return
        }
        guard let lastHeartbeatAck else {
            transportHealth = .degraded(reason: "Waiting for heartbeat")
            return
        }

        let age = Date().timeIntervalSince(lastHeartbeatAck)
        if age > Self.lostHeartbeatAge {
            transportHealth = .lost(reason: "Heartbeat timed out")
        } else if age > Self.degradedHeartbeatAge {
            transportHealth = .degraded(reason: "Heartbeat delayed")
        } else {
            transportHealth = .healthy
        }
    }

    private func updateReconnectState(
        connected: Bool,
        authenticated: Bool,
        sessionId: UInt32?,
        error: String?
    ) {
        if connected, (authenticated || sessionId != nil) {
            cancelReconnect()
            reconnectState = .idle
            return
        }
        guard reconnectAllowed, lastHost != nil, lastPort != nil else {
            reconnectState = .idle
            return
        }

        let reconnectReason: String? = {
            switch transportHealth {
            case .lost(let reason):
                return reason
            default:
                break
            }
            if let error, !connected {
                return error
            }
            if case .connectionLost(let reason) = status {
                return reason
            }
            return nil
        }()

        guard let reconnectReason else {
            if case .waiting = reconnectState {
                return
            }
            reconnectState = .idle
            return
        }

        guard reconnectAttempt < Self.maxReconnectAttempts else {
            reconnectState = .failed(reason: reconnectReason)
            reconnectAllowed = false
            cancelReconnect()
            return
        }

        if case .waiting = reconnectState {
            return
        }

        reconnectAttempt += 1
        reconnectState = .waiting(attempt: reconnectAttempt, nextRetryIn: Self.reconnectDelay)
        scheduleReconnect()
    }

    private func scheduleReconnect() {
        cancelReconnect()
        let workItem = DispatchWorkItem { [weak self] in
            guard let self,
                  self.reconnectAllowed,
                  let host = self.lastHost,
                  let port = self.lastPort else { return }
            self.status = .connecting
            self.client.connect(host: host, port: port, authKey: self.lastAuthKey ?? "")
        }
        reconnectWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.reconnectDelay, execute: workItem)
    }

    private func cancelReconnect() {
        reconnectWorkItem?.cancel()
        reconnectWorkItem = nil
        reconnectAttempt = 0
    }
}
