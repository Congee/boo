import Foundation
import Combine
import Network
import Security

let BooDefaultRemotePort: UInt16 = 7337
let BooDefaultRemotePortText = String(BooDefaultRemotePort)

struct SavedNode: Identifiable, Codable {
    var id = UUID()
    var name: String
    var host: String
    var port: UInt16 = BooDefaultRemotePort
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

struct TailscaleDiscoverySettings: Codable, Equatable {
    var defaultPort: UInt16 = BooDefaultRemotePort
}

struct TerminalDisplaySettings: Codable, Equatable {
    var showFloatingBackButton = true
}

private enum KeychainStringStore {
    private static func describe(_ status: OSStatus) -> String {
        if let message = SecCopyErrorMessageString(status, nil) as String? {
            return message
        }
        return "OSStatus \(status)"
    }

    static func load(service: String, account: String) throws -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw NSError(
                domain: "BooKeychain",
                code: Int(status),
                userInfo: [NSLocalizedDescriptionKey: "Failed to read Tailscale token from Keychain: \(describe(status))"]
            )
        }
        guard let data = item as? Data,
              let string = String(data: data, encoding: .utf8)
        else {
            throw NSError(
                domain: "BooKeychain",
                code: -1,
                userInfo: [NSLocalizedDescriptionKey: "Failed to decode Tailscale token from Keychain"]
            )
        }
        return string
    }

    static func save(_ value: String, service: String, account: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let data = Data(value.utf8)
        let attributes: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let updateStatus = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if updateStatus == errSecItemNotFound {
            var insert = query
            insert[kSecValueData as String] = data
            insert[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            let addStatus = SecItemAdd(insert as CFDictionary, nil)
            guard addStatus == errSecSuccess else {
                throw NSError(
                    domain: "BooKeychain",
                    code: Int(addStatus),
                    userInfo: [NSLocalizedDescriptionKey: "Failed to save Tailscale token to Keychain: \(describe(addStatus))"]
                )
            }
            return
        }
        guard updateStatus == errSecSuccess else {
            throw NSError(
                domain: "BooKeychain",
                code: Int(updateStatus),
                userInfo: [NSLocalizedDescriptionKey: "Failed to update Tailscale token in Keychain: \(describe(updateStatus))"]
            )
        }
    }

    static func delete(service: String, account: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw NSError(
                domain: "BooKeychain",
                code: Int(status),
                userInfo: [NSLocalizedDescriptionKey: "Failed to clear Tailscale token from Keychain: \(describe(status))"]
            )
        }
    }
}

func normalizeLegacyTabMetadataKeys(in data: Data) -> Data? {
    guard let jsonObject = try? JSONSerialization.jsonObject(with: data) else {
        return nil
    }
    let normalized = rewriteLegacyTabMetadataKeys(in: jsonObject)
    return try? JSONSerialization.data(withJSONObject: normalized)
}

private func rewriteLegacyTabMetadataKeys(in value: Any) -> Any {
    if let array = value as? [Any] {
        return array.map(rewriteLegacyTabMetadataKeys(in:))
    }
    if let dict = value as? [String: Any] {
        var rewritten: [String: Any] = [:]
        for (key, nestedValue) in dict {
            let rewrittenKey = key == "sessionId" ? "tabId" : key
            rewritten[rewrittenKey] = rewriteLegacyTabMetadataKeys(in: nestedValue)
        }
        return rewritten
    }
    return value
}

@MainActor
final class ConnectionStore: ObservableObject {
    @Published var savedNodes: [SavedNode] = []
    @Published var history: [ConnectionHistoryEntry] = []
    @Published var tailscaleDiscoverySettings = TailscaleDiscoverySettings()
    @Published var terminalDisplaySettings = TerminalDisplaySettings()
    @Published var tailscaleTokenStatusMessage: String?

    private let maxHistory = 50
    private let storageNamespaceSuffix: String
    private var trustedServerIdentities: [String: String] = [:]
    private var nodesKey: String { "boo.remote.savedNodes\(storageNamespaceSuffix)" }
    private var historyKey: String { "boo.remote.connectionHistory\(storageNamespaceSuffix)" }
    private var trustedIdentitiesKey: String { "boo.remote.trustedServerIdentities\(storageNamespaceSuffix)" }
    private var tailscaleSettingsKey: String { "boo.remote.tailscale.discovery\(storageNamespaceSuffix)" }
    private var terminalDisplaySettingsKey: String { "boo.remote.terminalDisplay\(storageNamespaceSuffix)" }
    private var tailscaleTokenService: String { "me.congee.boo.tailscale\(storageNamespaceSuffix)" }
    private let tailscaleTokenAccount = "api-token"

    init() {
        storageNamespaceSuffix = UITestLaunchConfiguration.current() == nil ? "" : ".uitest"
        applyUITestConfiguration()
        loadNodes()
        loadHistory()
        loadTrustedServerIdentities()
        clearLegacyHostTabStorage()
        loadTailscaleSettings()
        loadTerminalDisplaySettings()
        refreshTailscaleTokenStatus()
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

    var hasTailscaleAPIToken: Bool {
        guard let token = tailscaleAPIToken() else { return false }
        return !token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func tailscaleAPIToken() -> String? {
        try? KeychainStringStore.load(service: tailscaleTokenService, account: tailscaleTokenAccount)
    }

    func updateTailscaleDiscovery(defaultPort: UInt16) {
        tailscaleDiscoverySettings.defaultPort = defaultPort
        saveTailscaleSettings()
    }

    func updateTerminalDisplay(showFloatingBackButton: Bool) {
        terminalDisplaySettings.showFloatingBackButton = showFloatingBackButton
        saveTerminalDisplaySettings()
    }

    func refreshTailscaleTokenStatus() {
        guard let token = tailscaleAPIToken(),
              !token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            tailscaleTokenStatusMessage = nil
            return
        }
        tailscaleTokenStatusMessage = "Tailscale token saved securely in Keychain."
    }

    @discardableResult
    func replaceTailscaleAPIToken(_ apiToken: String) -> Bool {
        let trimmed = apiToken.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return false }
        do {
            try KeychainStringStore.save(trimmed, service: tailscaleTokenService, account: tailscaleTokenAccount)
            tailscaleTokenStatusMessage = "Tailscale token saved securely in Keychain."
            return true
        } catch {
            tailscaleTokenStatusMessage = error.localizedDescription
            return false
        }
    }

    func clearTailscaleAPIToken() {
        do {
            try KeychainStringStore.delete(service: tailscaleTokenService, account: tailscaleTokenAccount)
            tailscaleTokenStatusMessage = nil
        } catch {
            tailscaleTokenStatusMessage = error.localizedDescription
        }
    }

    private func loadNodes() {
        guard let data = UserDefaults.standard.data(forKey: nodesKey),
              let nodes = try? JSONDecoder().decode([SavedNode].self, from: data) else { return }
        let filtered = UITestLaunchConfiguration.current() == nil
            ? nodes.filter { !isLikelyUITestArtifactNode($0) }
            : nodes
        savedNodes = filtered
        if filtered.count != nodes.count {
            saveNodes()
        }
    }

    private func saveNodes() {
        guard let data = try? JSONEncoder().encode(savedNodes) else { return }
        UserDefaults.standard.set(data, forKey: nodesKey)
    }

    private func isLikelyUITestArtifactNode(_ node: SavedNode) -> Bool {
        guard node.port != BooDefaultRemotePort else { return false }
        return node.name == "Local Boo" || node.name == "UI Test Node"
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

    private func clearLegacyHostTabStorage() {
        let defaults = UserDefaults.standard
        defaults.removeObject(forKey: "boo.remote.hostTabs\(storageNamespaceSuffix)")
        defaults.removeObject(forKey: "boo.remote.hostSessions\(storageNamespaceSuffix)")
        defaults.removeObject(forKey: "boo.remote.resumeAttachments\(storageNamespaceSuffix)")
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

    private func loadTerminalDisplaySettings() {
        guard let data = UserDefaults.standard.data(forKey: terminalDisplaySettingsKey),
              let settings = try? JSONDecoder().decode(TerminalDisplaySettings.self, from: data) else { return }
        terminalDisplaySettings = settings
    }

    private func saveTerminalDisplaySettings() {
        guard let data = try? JSONEncoder().encode(terminalDisplaySettings) else { return }
        UserDefaults.standard.set(data, forKey: terminalDisplaySettingsKey)
    }

    private func applyUITestConfiguration() {
        guard let config = UITestLaunchConfiguration.current() else { return }

        if config.resetStorage {
            UserDefaults.standard.removeObject(forKey: nodesKey)
            UserDefaults.standard.removeObject(forKey: historyKey)
            UserDefaults.standard.removeObject(forKey: trustedIdentitiesKey)
            UserDefaults.standard.removeObject(forKey: "boo.remote.hostTabs\(storageNamespaceSuffix)")
            UserDefaults.standard.removeObject(forKey: "boo.remote.hostSessions\(storageNamespaceSuffix)")
            UserDefaults.standard.removeObject(forKey: tailscaleSettingsKey)
            UserDefaults.standard.removeObject(forKey: terminalDisplaySettingsKey)
            try? KeychainStringStore.delete(service: tailscaleTokenService, account: tailscaleTokenAccount)
        }

        if let tailscalePort = config.tailscalePort {
            tailscaleDiscoverySettings.defaultPort = tailscalePort
            saveTailscaleSettings()
        }

        if let showFloatingBackButton = config.showFloatingBackButton {
            terminalDisplaySettings.showFloatingBackButton = showFloatingBackButton
            saveTerminalDisplaySettings()
        }

        if let tailscaleToken = config.tailscaleToken?.trimmingCharacters(in: .whitespacesAndNewlines),
           !tailscaleToken.isEmpty
        {
            try? KeychainStringStore.save(tailscaleToken, service: tailscaleTokenService, account: tailscaleTokenAccount)
        }

        guard let host = config.host else { return }
        let node = SavedNode(
            name: config.nodeName ?? "UI Test Node",
            host: host,
            port: config.port
        )
        guard let data = try? JSONEncoder().encode([node]) else { return }
        UserDefaults.standard.set(data, forKey: nodesKey)
    }

    private func targetKey(host: String, port: UInt16) -> String {
        "\(host):\(port)"
    }
}

enum ConnectionStatus: Equatable {
    case disconnected
    case connecting
    case connected
    case authenticated
    case activeTab(tabId: UInt32)
    case connectionLost(reason: String)
}

extension ConnectionStatus {
    var activeTabId: UInt32? {
        if case .activeTab(let tabId) = self {
            return tabId
        }
        return nil
    }
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

enum ConnectionRouteKind: Equatable {
    case bonjourLAN
    case tailscale
    case manual

    var networkUnavailableMessage: String {
        switch self {
        case .bonjourLAN:
            return "Local network unavailable on this iPad"
        case .tailscale:
            return "Tailscale path unavailable on this iPad"
        case .manual:
            return "Network unavailable on this iPad"
        }
    }

    var hostUnreachableMessage: String {
        switch self {
        case .bonjourLAN:
            return "LAN host unreachable from this iPad"
        case .tailscale:
            return "Tailscale host unreachable from this iPad"
        case .manual:
            return "Host unreachable from this iPad"
        }
    }
}

enum NetworkPathState: Equatable {
    case unsatisfied
    case satisfied(isExpensive: Bool, usesWiFi: Bool, usesCellular: Bool, usesWired: Bool)

    var summaryMessage: String {
        switch self {
        case .unsatisfied:
            return "This iPad is currently offline"
        case .satisfied(let isExpensive, let usesWiFi, let usesCellular, let usesWired):
            if usesWiFi { return "Connected over Wi-Fi" }
            if usesWired { return "Connected over wired network" }
            if usesCellular { return isExpensive ? "Connected over cellular" : "Connected over network" }
            return isExpensive ? "Connected over metered network" : "Connected over network"
        }
    }

    var isSatisfied: Bool {
        if case .satisfied = self { return true }
        return false
    }
}

@MainActor
final class ConnectionMonitor: ObservableObject {
    @Published var status: ConnectionStatus = .disconnected
    @Published var transportHealth: TransportHealth = .idle
    @Published var reconnectState: ReconnectState = .idle
    @Published var networkPathState: NetworkPathState = .unsatisfied
    @Published var latencyMs: Double?
    @Published var estimatedPacketLossRate: Double?

    private let client: GSPClient
    private let store: ConnectionStore
    private var cancellables = Set<AnyCancellable>()
    private var heartbeatTimer: AnyCancellable?
    private var reconnectWorkItem: DispatchWorkItem?
    private let pathMonitor = NWPathMonitor()
    private let pathQueue = DispatchQueue(label: "boo-ios.network-path")

    private static let degradedHeartbeatAge: TimeInterval = 8
    private static let lostHeartbeatAge: TimeInterval = 15
    private static let reconnectDelay: TimeInterval = 2
    private static let maxReconnectAttempts = 5
    private static let postDisconnectConnectDelay: TimeInterval = 0.2

    private var reconnectAllowed = false
    private var reconnectAttempt = 0
    private var connectionIntentGeneration: UInt64 = 0
    private var lastDisconnectAt: Date?

    private(set) var lastEndpoint: NWEndpoint?
    private(set) var lastHost: String?
    private(set) var lastPort: UInt16?
    private(set) var lastDisplayName: String?
    private(set) var lastRouteKind: ConnectionRouteKind = .manual
    private(set) var currentHistoryId: UUID?
    private(set) var currentNodeId: UUID?

    init(client: GSPClient, store: ConnectionStore) {
        self.client = client
        self.store = store
        observe()
        observePath()
    }

    private func observe() {
        Publishers.CombineLatest(
            Publishers.CombineLatest4(
                client.$connected,
                client.$authenticated,
                client.$activeTabId,
                client.$lastError
            ),
            client.$lastHeartbeatAck
        )
        .receive(on: DispatchQueue.main)
        .sink { [weak self] values, lastHeartbeatAck in
            guard let self else { return }
            let (connected, authenticated, tabId, error) = values

            if let tabId {
                self.status = .activeTab(tabId: tabId)
            } else if let error, !connected, self.status != .disconnected {
                self.status = .connectionLost(reason: error)
            } else if authenticated {
                self.status = .authenticated
            } else if connected {
                self.status = .connected
            } else if self.lastHost != nil, case .activeTab = self.status {
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
                tabId: tabId,
                error: error
            )
        }
        .store(in: &cancellables)

        Publishers.CombineLatest3(
            client.$lastHeartbeatRttMs,
            client.$heartbeatSentCount,
            Publishers.CombineLatest(client.$heartbeatAckCount, client.$heartbeatTimeoutCount)
        )
        .receive(on: DispatchQueue.main)
        .sink { [weak self] rtt, sent, ackAndTimeout in
            guard let self else { return }
            let (acked, timeouts) = ackAndTimeout
            self.latencyMs = rtt
            if sent < 5 {
                self.estimatedPacketLossRate = nil
            } else {
                let outstanding = max(Int64(sent) - Int64(acked) - Int64(timeouts), 0)
                let lostEstimate = Double(timeouts) + Double(outstanding) * 0.5
                self.estimatedPacketLossRate = min(100, max(0, (lostEstimate / Double(sent)) * 100))
            }
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

    func connect(host: String, port: UInt16, routeKind: ConnectionRouteKind = .manual, displayName: String? = nil, historyId: UUID? = nil, nodeId: UUID? = nil) {
        connectionIntentGeneration &+= 1
        let intentGeneration = connectionIntentGeneration
        lastEndpoint = nil
        lastHost = host
        lastPort = port
        lastDisplayName = displayName
        lastRouteKind = routeKind
        currentHistoryId = historyId
        currentNodeId = nodeId
        status = .connecting
        transportHealth = .idle
        reconnectAllowed = true
        cancelReconnect()
        beginConnection(intentGeneration: intentGeneration) {
            self.startClientConnection(host: host, port: port)
        }
    }

    func connect(endpoint: NWEndpoint, displayHost: String, displayPort: UInt16, routeKind: ConnectionRouteKind = .manual, displayName: String? = nil, historyId: UUID? = nil, nodeId: UUID? = nil) {
        connectionIntentGeneration &+= 1
        let intentGeneration = connectionIntentGeneration
        lastEndpoint = endpoint
        lastHost = displayHost
        lastPort = displayPort
        lastDisplayName = displayName
        lastRouteKind = routeKind
        currentHistoryId = historyId
        currentNodeId = nodeId
        status = .connecting
        transportHealth = .idle
        reconnectAllowed = true
        cancelReconnect()
        beginConnection(intentGeneration: intentGeneration) {
            self.startClientConnection(endpoint: endpoint, host: displayHost, port: displayPort)
        }
    }

    private func beginConnection(intentGeneration: UInt64, start: @escaping () -> Void) {
        let delay = max(
            0,
            Self.postDisconnectConnectDelay - Date().timeIntervalSince(lastDisconnectAt ?? .distantPast)
        )
        guard delay > 0 else {
            start()
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self, self.connectionIntentGeneration == intentGeneration else { return }
            start()
        }
    }

    private func startClientConnection(host: String, port: UInt16) {
        client.configureTrustedServerIdentity(store.trustedServerIdentity(host: host, port: port))
        client.connect(host: host, port: port)
    }

    private func startClientConnection(endpoint: NWEndpoint, host: String, port: UInt16) {
        client.configureTrustedServerIdentity(store.trustedServerIdentity(host: host, port: port))
        client.connect(endpoint: endpoint)
    }

    func reconnect() {
        if let endpoint = lastEndpoint, let host = lastHost, let port = lastPort {
            connect(endpoint: endpoint, displayHost: host, displayPort: port)
            return
        }
        guard let host = lastHost, let port = lastPort else { return }
        connect(host: host, port: port)
    }

    func disconnect() {
        connectionIntentGeneration &+= 1
        lastDisconnectAt = Date()
        reconnectAllowed = false
        cancelReconnect()
        client.disconnect()
        status = .disconnected
        transportHealth = .idle
        reconnectState = .idle
        lastEndpoint = nil
        lastHost = nil
        lastPort = nil
        lastDisplayName = nil
        lastRouteKind = .manual
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
        tabId: UInt32?,
        error: String?
    ) {
        if connected, (authenticated || tabId != nil) {
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
        let intentGeneration = connectionIntentGeneration
        let workItem = DispatchWorkItem { [weak self] in
            guard let self,
                  self.connectionIntentGeneration == intentGeneration,
                  self.reconnectAllowed else { return }
            self.status = .connecting
            if let endpoint = self.lastEndpoint,
               let host = self.lastHost,
               let port = self.lastPort
            {
                self.startClientConnection(endpoint: endpoint, host: host, port: port)
            } else if let host = self.lastHost,
                      let port = self.lastPort
            {
                self.startClientConnection(host: host, port: port)
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

    func contextualErrorMessage(_ raw: String) -> String {
        switch raw {
        case "Network unavailable on this iPad":
            return lastRouteKind.networkUnavailableMessage
        case "Host unreachable from this iPad":
            return lastRouteKind.hostUnreachableMessage
        case "Host network unreachable from this iPad":
            return lastRouteKind.hostUnreachableMessage
        default:
            return raw
        }
    }

    var networkStatusSummary: String {
        networkPathState.summaryMessage
    }

    var latencyAndLossSummary: String? {
        let latencyPart = latencyMs.map { String(format: "%.0f ms", $0) }
        let lossPart = estimatedPacketLossRate.map { String(format: "%.0f%% loss", $0) }
        let pieces = [latencyPart, lossPart].compactMap { $0 }
        return pieces.isEmpty ? nil : pieces.joined(separator: " · ")
    }

    private func observePath() {
        pathMonitor.pathUpdateHandler = { [weak self] path in
            Task { @MainActor in
                guard let self else { return }
                if path.status != .satisfied {
                    self.networkPathState = .unsatisfied
                } else {
                    self.networkPathState = .satisfied(
                        isExpensive: path.isExpensive,
                        usesWiFi: path.usesInterfaceType(.wifi),
                        usesCellular: path.usesInterfaceType(.cellular),
                        usesWired: path.usesInterfaceType(.wiredEthernet)
                    )
                }
            }
        }
        pathMonitor.start(queue: pathQueue)
    }
}
