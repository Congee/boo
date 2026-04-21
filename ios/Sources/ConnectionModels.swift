import Foundation
import Combine
import Network
import Security

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

struct ResumeAttachmentMetadata: Codable, Equatable {
    var sessionId: UInt32
    var attachmentId: UInt64
    var resumeToken: UInt64
    var recordedAt: Date
}

struct TailscaleDiscoverySettings: Codable, Equatable {
    var defaultPort: UInt16 = 7337
}

private enum KeychainStringStore {
    static func load(service: String, account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess,
              let data = item as? Data,
              let string = String(data: data, encoding: .utf8)
        else {
            return nil
        }
        return string
    }

    static func save(_ value: String, service: String, account: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let data = Data(value.utf8)
        let attributes: [String: Any] = [
            kSecValueData as String: data,
        ]
        let updateStatus = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if updateStatus == errSecItemNotFound {
            var insert = query
            insert[kSecValueData as String] = data
            SecItemAdd(insert as CFDictionary, nil)
        }
    }

    static func delete(service: String, account: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }
}

@MainActor
final class ConnectionStore: ObservableObject {
    @Published var savedNodes: [SavedNode] = []
    @Published var history: [ConnectionHistoryEntry] = []
    @Published var tailscaleDiscoverySettings = TailscaleDiscoverySettings()

    private let nodesKey = "boo.remote.savedNodes"
    private let historyKey = "boo.remote.connectionHistory"
    private let trustedIdentitiesKey = "boo.remote.trustedServerIdentities"
    private let resumeAttachmentsKey = "boo.remote.resumeAttachments"
    private let tailscaleSettingsKey = "boo.remote.tailscale.discovery"
    private let tailscaleTokenService = "me.congee.boo.tailscale"
    private let tailscaleTokenAccount = "api-token"
    private let maxHistory = 50
    private var trustedServerIdentities: [String: String] = [:]
    private var resumeAttachments: [String: ResumeAttachmentMetadata] = [:]

    init() {
        applyUITestConfiguration()
        loadNodes()
        loadHistory()
        loadTrustedServerIdentities()
        loadResumeAttachments()
        loadTailscaleSettings()
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

    func recordTrustedServerIdentity(host: String, port: UInt16, identityId: String) -> String? {
        let key = "\(host):\(port)"
        if let existing = trustedServerIdentities[key] {
            guard existing == identityId else {
                return "Server identity changed for \(key). Expected \(existing), got \(identityId)."
            }
            return nil
        }
        trustedServerIdentities[key] = identityId
        saveTrustedServerIdentities()
        return nil
    }

    func trustedServerIdentity(host: String, port: UInt16) -> String? {
        trustedServerIdentities["\(host):\(port)"]
    }

    func trustServerIdentity(host: String, port: UInt16, identityId: String) {
        trustedServerIdentities["\(host):\(port)"] = identityId
        saveTrustedServerIdentities()
    }

    func recordResumeAttachment(host: String, port: UInt16, sessionId: UInt32, attachmentId: UInt64, resumeToken: UInt64) {
        resumeAttachments["\(host):\(port)"] = ResumeAttachmentMetadata(
            sessionId: sessionId,
            attachmentId: attachmentId,
            resumeToken: resumeToken,
            recordedAt: Date()
        )
        saveResumeAttachments()
    }

    func resumeAttachment(host: String, port: UInt16) -> ResumeAttachmentMetadata? {
        resumeAttachments["\(host):\(port)"]
    }

    func clearResumeAttachment(host: String, port: UInt16) {
        resumeAttachments.removeValue(forKey: "\(host):\(port)")
        saveResumeAttachments()
    }

    var hasTailscaleAPIToken: Bool {
        guard let token = tailscaleAPIToken() else { return false }
        return !token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func tailscaleAPIToken() -> String? {
        KeychainStringStore.load(service: tailscaleTokenService, account: tailscaleTokenAccount)
    }

    func updateTailscaleDiscovery(defaultPort: UInt16) {
        tailscaleDiscoverySettings.defaultPort = defaultPort
        saveTailscaleSettings()
    }

    func replaceTailscaleAPIToken(_ apiToken: String) {
        let trimmed = apiToken.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        KeychainStringStore.save(trimmed, service: tailscaleTokenService, account: tailscaleTokenAccount)
    }

    func clearTailscaleAPIToken() {
        KeychainStringStore.delete(service: tailscaleTokenService, account: tailscaleTokenAccount)
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

    private func loadTrustedServerIdentities() {
        guard let data = UserDefaults.standard.data(forKey: trustedIdentitiesKey),
              let instances = try? JSONDecoder().decode([String: String].self, from: data) else { return }
        trustedServerIdentities = instances
    }

    private func saveTrustedServerIdentities() {
        guard let data = try? JSONEncoder().encode(trustedServerIdentities) else { return }
        UserDefaults.standard.set(data, forKey: trustedIdentitiesKey)
    }

    private func loadResumeAttachments() {
        guard let data = UserDefaults.standard.data(forKey: resumeAttachmentsKey),
              let attachments = try? JSONDecoder().decode([String: ResumeAttachmentMetadata].self, from: data) else { return }
        resumeAttachments = attachments
    }

    private func saveResumeAttachments() {
        guard let data = try? JSONEncoder().encode(resumeAttachments) else { return }
        UserDefaults.standard.set(data, forKey: resumeAttachmentsKey)
    }

    private func loadTailscaleSettings() {
        guard let data = UserDefaults.standard.data(forKey: tailscaleSettingsKey),
              let settings = try? JSONDecoder().decode(TailscaleDiscoverySettings.self, from: data) else { return }
        tailscaleDiscoverySettings = settings
    }

    private func saveTailscaleSettings() {
        guard let data = try? JSONEncoder().encode(tailscaleDiscoverySettings) else { return }
        UserDefaults.standard.set(data, forKey: tailscaleSettingsKey)
    }

    private func applyUITestConfiguration() {
        guard let config = UITestLaunchConfiguration.current() else { return }

        if config.resetStorage {
            UserDefaults.standard.removeObject(forKey: nodesKey)
            UserDefaults.standard.removeObject(forKey: historyKey)
            UserDefaults.standard.removeObject(forKey: trustedIdentitiesKey)
            UserDefaults.standard.removeObject(forKey: resumeAttachmentsKey)
            UserDefaults.standard.removeObject(forKey: tailscaleSettingsKey)
            KeychainStringStore.delete(service: tailscaleTokenService, account: tailscaleTokenAccount)
        }

        guard let host = config.host else { return }
        let node = SavedNode(
            name: config.nodeName ?? "UI Test Node",
            host: host,
            port: config.port,
            authKey: config.authKey
        )
        guard let data = try? JSONEncoder().encode([node]) else { return }
        UserDefaults.standard.set(data, forKey: nodesKey)
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
    private let store: ConnectionStore
    private var cancellables = Set<AnyCancellable>()
    private var heartbeatTimer: AnyCancellable?
    private var reconnectWorkItem: DispatchWorkItem?

    private static let degradedHeartbeatAge: TimeInterval = 8
    private static let lostHeartbeatAge: TimeInterval = 15
    private static let reconnectDelay: TimeInterval = 2
    private static let maxReconnectAttempts = 5

    private var reconnectAllowed = false
    private var reconnectAttempt = 0

    private(set) var lastEndpoint: NWEndpoint?
    private(set) var lastHost: String?
    private(set) var lastPort: UInt16?
    private(set) var lastAuthKey: String?
    private(set) var currentHistoryId: UUID?
    private(set) var currentNodeId: UUID?

    init(client: GSPClient, store: ConnectionStore) {
        self.client = client
        self.store = store
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
        lastEndpoint = nil
        lastHost = host
        lastPort = port
        lastAuthKey = authKey
        currentHistoryId = historyId
        currentNodeId = nodeId
        status = .connecting
        transportHealth = .idle
        reconnectAllowed = true
        cancelReconnect()
        startClientConnection(host: host, port: port, authKey: authKey)
    }

    func connect(endpoint: NWEndpoint, displayHost: String, displayPort: UInt16, authKey: String = "", historyId: UUID? = nil, nodeId: UUID? = nil) {
        lastEndpoint = endpoint
        lastHost = displayHost
        lastPort = displayPort
        lastAuthKey = authKey
        currentHistoryId = historyId
        currentNodeId = nodeId
        status = .connecting
        transportHealth = .idle
        reconnectAllowed = true
        cancelReconnect()
        startClientConnection(endpoint: endpoint, host: displayHost, port: displayPort, authKey: authKey)
    }

    private func startClientConnection(host: String, port: UInt16, authKey: String) {
        client.configureTrustedServerIdentity(store.trustedServerIdentity(host: host, port: port))
        if let resume = store.resumeAttachment(host: host, port: port) {
            client.configureResumeAttachment(
                sessionId: resume.sessionId,
                attachmentId: resume.attachmentId,
                resumeToken: resume.resumeToken
            )
        }
        client.connect(host: host, port: port, authKey: authKey)
    }

    private func startClientConnection(endpoint: NWEndpoint, host: String, port: UInt16, authKey: String) {
        client.configureTrustedServerIdentity(store.trustedServerIdentity(host: host, port: port))
        if let resume = store.resumeAttachment(host: host, port: port) {
            client.configureResumeAttachment(
                sessionId: resume.sessionId,
                attachmentId: resume.attachmentId,
                resumeToken: resume.resumeToken
            )
        }
        client.connect(endpoint: endpoint, authKey: authKey)
    }

    func reconnect() {
        if let endpoint = lastEndpoint, let host = lastHost, let port = lastPort {
            connect(endpoint: endpoint, displayHost: host, displayPort: port, authKey: lastAuthKey ?? "")
            return
        }
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
        lastEndpoint = nil
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
                  self.reconnectAllowed else { return }
            self.status = .connecting
            if let endpoint = self.lastEndpoint,
               let host = self.lastHost,
               let port = self.lastPort
            {
                self.startClientConnection(endpoint: endpoint, host: host, port: port, authKey: self.lastAuthKey ?? "")
            } else if let host = self.lastHost,
                      let port = self.lastPort
            {
                self.startClientConnection(host: host, port: port, authKey: self.lastAuthKey ?? "")
            }
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
