import Foundation
import Network
import Security
import UIKit

enum GSPMessageType: UInt8 {
    case auth = 0x01
    case listTabs = 0x02
    case create = 0x05
    case input = 0x06
    case resize = 0x07
    case destroy = 0x08
    case scroll = 0x0a
    case appMouseEvent = 0x10
    case heartbeat = 0x11

    case authOk = 0x80
    case authFail = 0x81
    case tabList = 0x82
    case fullState = 0x83
    case delta = 0x84
    case errorMsg = 0x87
    case tabExited = 0x89
    case scrollData = 0x8a
    case clipboard = 0x8b
    case image = 0x8c
    case uiRuntimeState = 0x8d
    case uiAppearance = 0x8e
    case heartbeatAck = 0x92
}

private struct OutboundWheelScrolledLinesPayload: Encodable {
    let x: Double
    let y: Double
    let mods: Int32
}

private enum OutboundAppMouseEvent: Encodable {
    case wheelScrolledLines(OutboundWheelScrolledLinesPayload)

    private enum CodingKeys: String, CodingKey {
        case WheelScrolledLines
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .wheelScrolledLines(let payload):
            try container.encode(payload, forKey: .WheelScrolledLines)
        }
    }
}

struct WireCell {
    var codepoint: UInt32 = 0
    var fg_r: UInt8 = 0
    var fg_g: UInt8 = 0
    var fg_b: UInt8 = 0
    var bg_r: UInt8 = 0
    var bg_g: UInt8 = 0
    var bg_b: UInt8 = 0
    var styleFlags: UInt8 = 0
    var wide: UInt8 = 0

    var hasFg: Bool { (styleFlags & 0x20) != 0 }
    var hasBg: Bool { (styleFlags & 0x40) != 0 }
    var isBold: Bool { (styleFlags & 0x01) != 0 }
    var isItalic: Bool { (styleFlags & 0x02) != 0 }
}

@MainActor
final class ScreenState: ObservableObject {
    @Published var rows: UInt16 = 0
    @Published var cols: UInt16 = 0
    @Published var cells: [WireCell] = []
    @Published var cursorX: UInt16 = 0
    @Published var cursorY: UInt16 = 0
    @Published var cursorVisible: Bool = true
    @Published var cursorBlinking: Bool = false
    @Published var cursorStyle: Int32 = 0

    func getCell(col: Int, row: Int) -> WireCell {
        let index = row * Int(cols) + col
        guard index >= 0, index < cells.count else { return WireCell() }
        return cells[index]
    }

    var accessibilityTextSnapshot: String {
        guard rows > 0, cols > 0, cells.count == Int(rows) * Int(cols) else { return "" }

        var text = ""
        for row in 0..<Int(rows) {
            for col in 0..<Int(cols) {
                let index = row * Int(cols) + col
                let codepoint = cells[index].codepoint
                if codepoint == 0 {
                    text.append(" ")
                } else if let scalar = UnicodeScalar(codepoint) {
                    text.append(Character(scalar))
                }
            }
            if row + 1 < Int(rows) {
                text.append("\n")
            }
        }
        return text
    }
}

struct DiscoveredDaemon: Identifiable, Hashable {
    let id: String
    let name: String
    let title: String
    let subtitle: String
    let endpoint: NWEndpoint
}

struct RemoteRuntimeTabSnapshot: Decodable, Equatable {
    let tabId: UInt32
    let index: Int
    let active: Bool
    let title: String
    let paneCount: Int
}

struct RemoteRuntimeStateSnapshot: Decodable, Equatable {
    let activeTab: Int
    let focusedPane: UInt64
    let tabs: [RemoteRuntimeTabSnapshot]
    let pwd: String
}

func decodeRemoteRuntimeState(_ payload: Data) -> RemoteRuntimeStateSnapshot? {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return try? decoder.decode(RemoteRuntimeStateSnapshot.self, from: payload)
}

struct TailscalePeer: Identifiable, Hashable {
    let id: String
    let name: String
    let host: String
    let port: UInt16
    let address: String?
    let os: String?
    let online: Bool
    let lastSeen: Date?

    private static let relativeFormatter: RelativeDateTimeFormatter = {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return formatter
    }()

    var stateDescription: String {
        if online {
            return "online"
        }
        if let lastSeen {
            let relative = Self.relativeFormatter.localizedString(for: lastSeen, relativeTo: Date())
            return "offline, last seen \(relative)"
        }
        return "offline"
    }
}

enum TailscalePeerProbeStatus: Equatable {
    case probing
    case reachable
    case unreachable
}

enum BooPortProbeStatus: Equatable {
    case probing
    case open
    case closed
}

struct TailscalePeerProbeMetrics: Equatable {
    let hostStatus: TailscalePeerProbeStatus
    let latencyMs: Double?
    let lossRate: Double?
    let portStatus: BooPortProbeStatus
}

func measureBooQUICHandshakeLatency(endpoint: NWEndpoint) async -> Double? {
    await withCheckedContinuation { continuation in
        let queue = DispatchQueue(label: "boo-ios.endpoint-probe.\(endpoint)")
        let options = NWProtocolQUIC.Options(alpn: ["boo-remote"])
        options.direction = .bidirectional
        options.idleTimeout = 12_000
        sec_protocol_options_set_verify_block(options.securityProtocolOptions, { _, _, complete in
            complete(true)
        }, queue)
        let params = NWParameters(quic: options)
        params.allowLocalEndpointReuse = true
        params.includePeerToPeer = true
        let connection = NWConnection(to: endpoint, using: params)
        let start = Date()
        var finished = false

        @Sendable func resolve(_ value: Double?) {
            guard !finished else { return }
            finished = true
            connection.stateUpdateHandler = nil
            connection.cancel()
            continuation.resume(returning: value)
        }

        let timeout = DispatchSource.makeTimerSource(queue: queue)
        timeout.schedule(deadline: .now() + 3)
        timeout.setEventHandler {
            timeout.cancel()
            resolve(nil)
        }
        timeout.resume()

        connection.stateUpdateHandler = { state in
            switch state {
            case .ready:
                timeout.cancel()
                resolve(Date().timeIntervalSince(start) * 1000)
            case .failed, .cancelled:
                timeout.cancel()
                resolve(nil)
            default:
                break
            }
        }
        connection.start(queue: queue)
    }
}

func measureBooQUICHandshakeLatency(host: String, port: UInt16) async -> Double? {
    guard let endpointPort = NWEndpoint.Port(rawValue: port) else {
        return nil
    }
    return await measureBooQUICHandshakeLatency(
        endpoint: .hostPort(host: NWEndpoint.Host(host), port: endpointPort)
    )
}

@MainActor
final class BonjourBrowser: ObservableObject {
    @Published var daemons: [DiscoveredDaemon] = []
    @Published var isSearching = false
    @Published var lastError: String?

    private var browsers: [NWBrowser] = []
    private let queue = DispatchQueue(label: "boo-bonjour-browser")
    private let serviceTypes = ["_boo._udp"]

    private struct RankedDiscoveredDaemon {
        let daemon: DiscoveredDaemon
        let advertisedPort: UInt16?
    }

    private func stripBonjourConflictSuffix(_ name: String) -> String {
        guard
            let regex = try? NSRegularExpression(pattern: #"^(.*?)(?: \((\d+)\))$"#),
            let match = regex.firstMatch(in: name, range: NSRange(name.startIndex..., in: name)),
            match.numberOfRanges == 3,
            let baseRange = Range(match.range(at: 1), in: name)
        else {
            return name
        }

        let base = String(name[baseRange])
        if base == "boo" || base.starts(with: "boo on ") || base.contains(" (") {
            return base
        }
        return name
    }

    private func normalizedServiceName(for endpoint: NWEndpoint) -> String {
        switch endpoint {
        case .service(let name, _, _, _):
            return stripBonjourConflictSuffix(name)
        default:
            return "\(endpoint)"
        }
    }

    private func displayTitle(for serviceName: String) -> String {
        let cleanedName = serviceName
            .replacingOccurrences(of: "boo on ", with: "")
            .replacingOccurrences(of: ".local", with: "")
        return cleanedName
            .replacingOccurrences(of: #" \((\d+)\)$"#, with: "", options: .regularExpression)
    }

    private func advertisedPort(for serviceName: String) -> UInt16? {
        guard
            let regex = try? NSRegularExpression(pattern: #"^boo on .+ \((\d+)\)$"#),
            let match = regex.firstMatch(in: serviceName, range: NSRange(serviceName.startIndex..., in: serviceName)),
            match.numberOfRanges == 2,
            let portRange = Range(match.range(at: 1), in: serviceName)
        else {
            return nil
        }
        return UInt16(serviceName[portRange])
    }

    private enum BrowserErrorDisposition {
        case ignore
        case show(String)
    }

    private func browserErrorDisposition(for error: NWError) -> BrowserErrorDisposition {
        let raw = "\(error)"
        if raw.contains("NoAuth") {
            return .show("Local network access is required for Bonjour discovery. Enable boo in Settings > Privacy & Security > Local Network.")
        }
        if raw.contains("DefunctConnection") {
            return .ignore
        }
        return .show("Bonjour browse failed: \(error)")
    }

    func startBrowsing() {
        stopBrowsing()
        isSearching = true
        lastError = nil
        for type in serviceTypes {
            let descriptor = NWBrowser.Descriptor.bonjour(type: type, domain: nil)
            let params = NWParameters.udp
            params.includePeerToPeer = true
            let browser = NWBrowser(for: descriptor, using: params)
            browser.stateUpdateHandler = { [weak self] state in
                Task { @MainActor in
                    switch state {
                    case .failed(let error):
                        switch self?.browserErrorDisposition(for: error) {
                        case .ignore:
                            self?.lastError = nil
                        case .show(let message):
                            self?.isSearching = false
                            self?.lastError = message
                        case .none:
                            break
                        }
                    case .waiting(let error):
                        switch self?.browserErrorDisposition(for: error) {
                        case .ignore:
                            self?.lastError = nil
                        case .show(let message):
                            self?.lastError = message
                        case .none:
                            break
                        }
                    case .ready:
                        self?.lastError = nil
                    case .cancelled:
                        self?.isSearching = false
                    default:
                        break
                    }
                }
            }
            browser.browseResultsChangedHandler = { [weak self] _, _ in
                Task { @MainActor in
                    self?.refreshDiscoveredDaemons()
                }
            }
            browser.start(queue: queue)
            browsers.append(browser)
        }
    }

    func stopBrowsing() {
        browsers.forEach { $0.cancel() }
        browsers.removeAll()
        daemons.removeAll()
        isSearching = false
        lastError = nil
    }

    private func refreshDiscoveredDaemons() {
        Task { @MainActor in
            var entriesByTitle: [String: RankedDiscoveredDaemon] = [:]
            for browser in browsers {
                for result in browser.browseResults {
                    let id = normalizedServiceName(for: result.endpoint)
                    let name: String
                    switch result.endpoint {
                    case .service(let n, _, _, _):
                        name = stripBonjourConflictSuffix(n)
                    default:
                        name = id
                    }
                    let title = displayTitle(for: name)
                    let subtitle: String
                    switch result.endpoint {
                    case .service(_, _, _, let interface):
                        if let interface {
                            subtitle = "QUIC remote daemon · \(interface.debugDescription)"
                        } else {
                            subtitle = "QUIC remote daemon"
                        }
                    default:
                        subtitle = "QUIC remote daemon"
                    }
                    let ranked = RankedDiscoveredDaemon(
                        daemon: DiscoveredDaemon(
                            id: id,
                            name: name,
                            title: title,
                            subtitle: subtitle,
                            endpoint: result.endpoint
                        ),
                        advertisedPort: advertisedPort(for: name)
                    )
                    if let existing = entriesByTitle[title] {
                        let existingScore = existing.advertisedPort == BooDefaultRemotePort ? 1 : 0
                        let candidateScore = ranked.advertisedPort == BooDefaultRemotePort ? 1 : 0
                        if candidateScore > existingScore {
                            entriesByTitle[title] = ranked
                        }
                    } else {
                        entriesByTitle[title] = ranked
                    }
                }
            }
            daemons = entriesByTitle.values
                .map(\.daemon)
                .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
            isSearching = !browsers.isEmpty
        }
    }
}

@MainActor
final class TailscalePeerBrowser: ObservableObject {
    @Published var peers: [TailscalePeer] = []
    @Published var isLoading = false
    @Published var lastError: String?
    @Published private(set) var probeMetrics: [String: TailscalePeerProbeMetrics] = [:]

    private var refreshTask: Task<Void, Never>?
    private var probeTasks: [String: Task<Void, Never>] = [:]
    private var probeState: [String: ProbeAccumulator] = [:]

    private struct ProbeAccumulator {
        var attempts: Int = 0
        var failures: Int = 0
        var successes: Int = 0
        var consecutiveFailures: Int = 0
        var lastLatencyMs: Double?
        var portAttempted = false
        var lastPortReachable = false

        var metrics: TailscalePeerProbeMetrics {
            let hostStatus: TailscalePeerProbeStatus = {
                if lastLatencyMs != nil {
                    return .reachable
                }
                if attempts > 0 && consecutiveFailures > 0 {
                    return .unreachable
                }
                return .probing
            }()
            let loss: Double? = {
                guard attempts >= 5, successes > 0 else { return nil }
                return (Double(failures) / Double(attempts)) * 100
            }()
            let portStatus: BooPortProbeStatus = {
                guard portAttempted else { return .probing }
                return lastPortReachable ? .open : .closed
            }()
            return TailscalePeerProbeMetrics(
                hostStatus: hostStatus,
                latencyMs: lastLatencyMs,
                lossRate: loss,
                portStatus: portStatus
            )
        }
    }

    func refresh(store: ConnectionStore) {
        refreshTask?.cancel()

        if let config = UITestLaunchConfiguration.current(), !config.mockTailscaleDevices.isEmpty {
            peers = config.mockTailscaleDevices.map {
                TailscalePeer(
                    id: "\($0.name)-\($0.host)",
                    name: $0.name,
                    host: $0.address ?? $0.host,
                    port: store.tailscaleDiscoverySettings.defaultPort,
                    address: $0.address,
                    os: $0.os,
                    online: $0.online,
                    lastSeen: nil
                )
            }
            lastError = nil
            isLoading = false
            updateProbeTasks()
            return
        }

        guard let token = store.tailscaleAPIToken(),
              !token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            peers = []
            lastError = nil
            isLoading = false
            clearProbeTasks()
            return
        }

        let port = store.tailscaleDiscoverySettings.defaultPort
        isLoading = true
        lastError = nil

        refreshTask = Task {
            do {
                let fetched = try await fetchPeers(token: token, port: port)
                guard !Task.isCancelled else { return }
                peers = fetched
                lastError = nil
                isLoading = false
                updateProbeTasks()
            } catch {
                guard !Task.isCancelled else { return }
                peers = []
                lastError = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
                isLoading = false
                clearProbeTasks()
            }
        }
    }

    func stop() {
        refreshTask?.cancel()
        refreshTask = nil
        isLoading = false
        clearProbeTasks()
    }

    private func fetchPeers(token: String, port: UInt16) async throws -> [TailscalePeer] {
        var request = URLRequest(url: URL(string: "https://api.tailscale.com/api/v2/tailnet/-/devices")!)
        let credential = Data("\(token):".utf8).base64EncodedString()
        request.setValue("Basic \(credential)", forHTTPHeaderField: "Authorization")
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw NSError(domain: "BooTailscale", code: -1, userInfo: [NSLocalizedDescriptionKey: "Tailscale API returned no HTTP response"])
        }
        guard (200...299).contains(http.statusCode) else {
            let body = String(data: data, encoding: .utf8) ?? "HTTP \(http.statusCode)"
            throw NSError(domain: "BooTailscale", code: http.statusCode, userInfo: [NSLocalizedDescriptionKey: "Tailscale API error: \(body)"])
        }
        return try parseTailscalePeers(data: data, port: port)
    }

    private func parseTailscalePeers(data: Data, port: UInt16) throws -> [TailscalePeer] {
        let object = try JSONSerialization.jsonObject(with: data)
        let deviceObjects: [[String: Any]]
        if let dict = object as? [String: Any], let devices = dict["devices"] as? [[String: Any]] {
            deviceObjects = devices
        } else if let devices = object as? [[String: Any]] {
            deviceObjects = devices
        } else {
            throw NSError(domain: "BooTailscale", code: -2, userInfo: [NSLocalizedDescriptionKey: "Unexpected Tailscale devices payload"])
        }

        let parsed: [TailscalePeer] = deviceObjects.compactMap { device in
            let rawName = (device["name"] as? String)?.trimmingCharacters(in: CharacterSet(charactersIn: "."))
            let hostname = (device["hostname"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
            let dnsName = (device["dnsName"] as? String)?.trimmingCharacters(in: CharacterSet(charactersIn: "."))
            let machineName = nonEmptyString(
                device["machineName"] as? String,
                device["computedName"] as? String,
                device["displayName"] as? String,
                device["nodeName"] as? String
            )
            let addresses = device["addresses"] as? [String] ?? []
            let preferredAddress = addresses.first(where: { $0.contains(".") }) ?? addresses.first
            let cleanedDisplayName = nonLocalName(machineName)
                ?? firstLabel(of: dnsName)
                ?? nonLocalName(hostname)
                ?? nonLocalName(rawName)
                ?? preferredAddress
            let connectHost = preferredAddress
                ?? dnsName
                ?? nonLocalName(rawName)
                ?? nonLocalName(hostname)
            guard let name = cleanedDisplayName, !name.isEmpty,
                  let host = connectHost, !host.isEmpty
            else {
                return nil
            }

            let lastSeen = parseTailscaleLastSeen(device)
            let online = parseTailscaleOnline(device, lastSeen: lastSeen)
            let os = device["os"] as? String
            let idValue = device["id"] ?? device["nodeId"] ?? host

            return TailscalePeer(
                id: String(describing: idValue),
                name: name,
                host: host,
                port: port,
                address: preferredAddress,
                os: os,
                online: online,
                lastSeen: lastSeen
            )
        }

        return parsed.sorted { lhs, rhs in
            if lhs.online != rhs.online {
                return lhs.online && !rhs.online
            }
            return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
        }
    }

    private func nonEmptyString(_ values: String?...) -> String? {
        guard let value = values.first(where: { value in
            guard let value else { return false }
            return !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }) ?? nil else {
            return nil
        }
        return value.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func firstLabel(of host: String?) -> String? {
        guard let host = host?.trimmingCharacters(in: CharacterSet(charactersIn: ".")),
              !host.isEmpty else {
            return nil
        }
        return String(host.split(separator: ".").first ?? "")
    }

    private func nonLocalName(_ value: String?) -> String? {
        guard let trimmed = value?.trimmingCharacters(in: CharacterSet(charactersIn: ".")).trimmingCharacters(in: .whitespacesAndNewlines),
              !trimmed.isEmpty else {
            return nil
        }
        if trimmed.compare("localhost", options: [.caseInsensitive, .diacriticInsensitive]) == .orderedSame {
            return nil
        }
        return firstLabel(of: trimmed)
    }

    private func parseTailscaleOnline(_ device: [String: Any], lastSeen: Date?) -> Bool {
        for key in ["online", "Online", "isOnline", "connected"] {
            if let value = device[key] as? Bool {
                return value
            }
        }
        if let lastSeen {
            return Date().timeIntervalSince(lastSeen) < 300
        }
        return true
    }

    private func parseTailscaleLastSeen(_ device: [String: Any]) -> Date? {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]

        for key in ["lastSeen", "LastSeen"] {
            if let value = device[key] as? String {
                if let date = formatter.date(from: value) {
                    return date
                }
                formatter.formatOptions = [.withInternetDateTime]
                if let date = formatter.date(from: value) {
                    return date
                }
                formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            }
        }
        return nil
    }

    private func updateProbeTasks() {
        let onlineIDs = Set(peers.filter(\.online).map(\.id))

        for (id, task) in probeTasks where !onlineIDs.contains(id) {
            task.cancel()
            probeTasks.removeValue(forKey: id)
            probeState.removeValue(forKey: id)
            probeMetrics.removeValue(forKey: id)
        }

        for peer in peers where peer.online {
            guard probeTasks[peer.id] == nil else { continue }
            probeTasks[peer.id] = Task { [weak self] in
                await self?.runProbeLoop(for: peer)
            }
        }
    }

    private func clearProbeTasks() {
        probeTasks.values.forEach { $0.cancel() }
        probeTasks.removeAll()
        probeState.removeAll()
        probeMetrics.removeAll()
    }

    private func runProbeLoop(for peer: TailscalePeer) async {
        while !Task.isCancelled {
            let latency = await measureBooQUICHandshakeLatency(host: peer.host, port: peer.port)
            if Task.isCancelled { return }
            await MainActor.run {
                var state = self.probeState[peer.id] ?? ProbeAccumulator()
                state.attempts += 1
                if let latency {
                    state.successes += 1
                    state.consecutiveFailures = 0
                    state.lastLatencyMs = latency
                    state.lastPortReachable = true
                } else {
                    state.failures += 1
                    state.consecutiveFailures += 1
                    state.lastLatencyMs = nil
                    state.lastPortReachable = false
                }
                state.portAttempted = true
                self.probeState[peer.id] = state
                self.probeMetrics[peer.id] = state.metrics
            }
            try? await Task.sleep(for: .seconds(15))
        }
    }
}

@MainActor
final class GSPClient: ObservableObject {
    private static let heartbeatInterval: TimeInterval = 5
    private static let heartbeatTimeout: TimeInterval = 12
    private static let connectionTimeout: TimeInterval = 10

    @Published var connected = false
    @Published var authenticated = false
    @Published var protocolVersion: UInt16?
    @Published var transportCapabilities: UInt32 = 0
    @Published var serverBuildId: String?
    @Published var serverInstanceId: String?
    @Published var serverIdentityId: String?
    @Published var lastSeenServerInstanceId: String?
    @Published var lastSeenServerIdentityId: String?
    @Published var lastHeartbeatAck: Date?
    @Published var lastHeartbeatRttMs: Double?
    @Published var heartbeatSentCount: UInt64 = 0
    @Published var heartbeatAckCount: UInt64 = 0
    @Published var heartbeatTimeoutCount: UInt64 = 0
    @Published var lastConnectLatencyMs: Double?
    @Published var lastAuthLatencyMs: Double?
    @Published var lastTabListLatencyMs: Double?
    @Published var connectionAttemptCount: UInt32 = 0
    @Published var reconnectAttemptCount: UInt32 = 0
    @Published var connectionDebugGeneration: UInt64 = 0
    @Published var tabs: [RemoteTabInfo] = []
    @Published var runtimeState: RemoteRuntimeStateSnapshot?
    @Published var screen = ScreenState()
    @Published var activeTabId: UInt32?
    @Published var lastErrorKind: ClientWireErrorKind?
    @Published var lastError: String?

    private var connection: NWConnection?
    private let queue = DispatchQueue(label: "boo-gsp-client", qos: .userInteractive)
    private var heartbeatTimer: DispatchSourceTimer?
    private var connectionTimeoutTimer: DispatchSourceTimer?
    private var lastHeartbeatSent: Date?
    private var pendingHeartbeatToken: UInt64?
    private var expectedServerIdentityId: String?
    private var connectionGeneration: UInt64 = 0
    private var pendingHostTabCreation = false
    private var autoTabBootstrapSuppressed = false
    private var connectStartedAt: Date?
    private var authRequestedAt: Date?
    private var tabListRequestedAt: Date?

    private func debugLog(_ message: String) {
        print("[boo-ios] \(message)")
    }

    private nonisolated static let magic: [UInt8] = [0x47, 0x53]
    private nonisolated static let headerLen = 7

    private var runtimeActiveTabId: UInt32? {
        guard let runtimeState,
              runtimeState.tabs.indices.contains(runtimeState.activeTab) else {
            return nil
        }
        return runtimeState.tabs[runtimeState.activeTab].tabId
    }

    var handshakeSummary: String? {
        guard let protocolVersion,
              let serverBuildId, !serverBuildId.isEmpty,
              let serverInstanceId, !serverInstanceId.isEmpty,
              let serverIdentityId, !serverIdentityId.isEmpty else {
            return nil
        }
        let heartbeat = lastHeartbeatRttMs.map { String(format: "hb %.0fms", $0) }
        let connect = lastConnectLatencyMs.map { String(format: "conn %.0fms", $0) }
        let auth = lastAuthLatencyMs.map { String(format: "auth %.0fms", $0) }
        let listed = lastTabListLatencyMs.map { String(format: "tabs %.0fms", $0) }
        let base = [ "proto \(protocolVersion)",
                     "caps 0x\(String(transportCapabilities, radix: 16))",
                     "gen \(connectionDebugGeneration)",
                     "conn# \(connectionAttemptCount)",
                     reconnectAttemptCount > 0 ? "reconn# \(reconnectAttemptCount)" : nil,
                     serverBuildId,
                     "id \(serverIdentityId)",
                     "srv \(serverInstanceId)"].compactMap { $0 }.joined(separator: " · ")
        let timings = [connect, auth, listed, heartbeat].compactMap { $0 }
        if !timings.isEmpty {
            return "\(base) · \(timings.joined(separator: " · "))"
        }
        return base
    }

    var uiTestTabDebugSummary: String {
        let tabIds = tabs.map(\.id).map(String.init).joined(separator: ",")
        return [
            "connected=\(connected)",
            "authenticated=\(authenticated)",
            "active=\(activeTabId.map(String.init) ?? "nil")",
            "runtimeActive=\(runtimeActiveTabId.map(String.init) ?? "nil")",
            "pendingCreate=\(pendingHostTabCreation)",
            "suppressed=\(autoTabBootstrapSuppressed)",
            "tabs=[\(tabIds)]",
            "lastError=\(lastError ?? "<none>")",
        ].joined(separator: " ")
    }

    func connect(host: String, port: UInt16) {
        prepareForConnectionAttempt()
        let generation = connectionGeneration
        let params = makeQUICParameters()
        connection = NWConnection(host: NWEndpoint.Host(host), port: NWEndpoint.Port(rawValue: port)!, using: params)
        installStateHandler(generation: generation)
        connection?.start(queue: queue)
    }

    func disconnect() {
        connection?.cancel()
        connection = nil
        stopConnectionTimeout()
        connectionGeneration &+= 1
        connected = false
        authenticated = false
        protocolVersion = nil
        transportCapabilities = 0
        serverBuildId = nil
        serverInstanceId = nil
        serverIdentityId = nil
        lastHeartbeatAck = nil
        lastHeartbeatRttMs = nil
        heartbeatSentCount = 0
        heartbeatAckCount = 0
        heartbeatTimeoutCount = 0
        lastHeartbeatSent = nil
        pendingHeartbeatToken = nil
        activeTabId = nil
        pendingHostTabCreation = false
        autoTabBootstrapSuppressed = false
        tabs = []
        runtimeState = nil
        screen = ScreenState()
        stopHeartbeatLoop()
        connectStartedAt = nil
        authRequestedAt = nil
        tabListRequestedAt = nil
        lastErrorKind = nil
        lastError = nil
    }

    func listTabs() {
        debugLog("send listTabs")
        tabListRequestedAt = Date()
        sendMessage(type: .listTabs, payload: Data())
    }

    func createTab(cols: UInt16 = 120, rows: UInt16 = 36) {
        debugLog("send createTab cols=\(cols) rows=\(rows)")
        pendingHostTabCreation = true
        var payload = Data(count: 4)
        payload.withUnsafeMutableBytes { buf in
            buf.storeBytes(of: cols.littleEndian, as: UInt16.self)
            buf.storeBytes(of: rows.littleEndian, toByteOffset: 2, as: UInt16.self)
        }
        sendMessage(type: .create, payload: payload)
    }

    func destroyTab(tabId: UInt32) {
        debugLog("send destroyTab id=\(tabId)")
        var payload = Data(count: 4)
        payload.withUnsafeMutableBytes { buf in
            buf.storeBytes(of: tabId.littleEndian, as: UInt32.self)
        }
        sendMessage(type: .destroy, payload: payload)
    }

    func suppressAutomaticTabBootstrap() {
        debugLog("suppressAutomaticTabBootstrap")
        autoTabBootstrapSuppressed = true
        pendingHostTabCreation = false
    }

    func configureTrustedServerIdentity(_ identityId: String?) {
        expectedServerIdentityId = identityId
    }

    func clearErrorState() {
        lastErrorKind = nil
        lastError = nil
    }

    func sendInput(_ text: String) {
        guard let data = text.data(using: .utf8) else { return }
        sendMessage(type: .input, payload: data)
    }

    func sendInputBytes(_ data: Data) {
        sendMessage(type: .input, payload: data)
    }

    func sendResize(cols: UInt16, rows: UInt16) {
        var payload = Data(count: 4)
        payload.withUnsafeMutableBytes { buf in
            buf.storeBytes(of: cols.littleEndian, as: UInt16.self)
            buf.storeBytes(of: rows.littleEndian, toByteOffset: 2, as: UInt16.self)
        }
        sendMessage(type: .resize, payload: payload)
    }

    func sendMouseWheelLines(x: Double = 0, y: Double, mods: Int32 = 0) {
        let event = OutboundAppMouseEvent.wheelScrolledLines(
            OutboundWheelScrolledLinesPayload(x: x, y: y, mods: mods)
        )
        guard let payload = try? JSONEncoder().encode(event) else { return }
        sendMessage(type: .appMouseEvent, payload: payload)
    }

    func sendHeartbeat() {
        let token = UInt64(Date().timeIntervalSince1970 * 1000)
        pendingHeartbeatToken = token
        heartbeatSentCount &+= 1
        sendMessage(type: .heartbeat, payload: Data(withUnsafeBytes(of: token.littleEndian, Array.init)))
    }

    private func startHeartbeatLoop() {
        stopHeartbeatLoop()
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + Self.heartbeatInterval, repeating: Self.heartbeatInterval)
        timer.setEventHandler { [weak self] in
            Task { @MainActor in
                self?.heartbeatTick()
            }
        }
        heartbeatTimer = timer
        timer.resume()
    }

    private func stopHeartbeatLoop() {
        heartbeatTimer?.cancel()
        heartbeatTimer = nil
    }

    private func heartbeatTick() {
        guard connected, authenticated, connection != nil else { return }
        let now = Date()
        if let sentAt = lastHeartbeatSent,
           lastHeartbeatAck.map({ $0 < sentAt }) ?? true,
           now.timeIntervalSince(sentAt) > Self.heartbeatTimeout {
            protocolError("Remote heartbeat timed out")
            return
        }
        sendHeartbeat()
        lastHeartbeatSent = now
    }

    private func sendAuth() {
        authRequestedAt = Date()
        sendMessage(type: .auth, payload: Data())
    }

    func connect(endpoint: NWEndpoint) {
        prepareForConnectionAttempt()
        let generation = connectionGeneration
        let params = makeQUICParameters()
        connection = NWConnection(to: endpoint, using: params)
        installStateHandler(generation: generation)
        connection?.start(queue: queue)
    }

    private func makeQUICParameters() -> NWParameters {
        let options = NWProtocolQUIC.Options(alpn: ["boo-remote"])
        options.direction = .bidirectional
        options.idleTimeout = Int(Self.heartbeatTimeout * 1000)
        sec_protocol_options_set_verify_block(options.securityProtocolOptions, { _, _, complete in
            complete(true)
        }, queue)
        let params = NWParameters(quic: options)
        params.allowLocalEndpointReuse = true
        params.includePeerToPeer = true
        return params
    }

    private func sendMessage(type: GSPMessageType, payload: Data) {
        let generation = connectionGeneration
        var header = Data(count: Self.headerLen)
        header[0] = Self.magic[0]
        header[1] = Self.magic[1]
        header[2] = type.rawValue
        let len = UInt32(payload.count).littleEndian
        header.withUnsafeMutableBytes { $0.storeBytes(of: len, toByteOffset: 3, as: UInt32.self) }
        connection?.send(
            content: header + payload,
            contentContext: .defaultStream,
            isComplete: false,
            completion: .contentProcessed { [weak self] error in
                guard let error else { return }
                Task { @MainActor in
                    guard let self, self.connectionGeneration == generation else { return }
                    self.lastError = "Send failed: \(error)"
                }
            }
        )
    }

    private func prepareForConnectionAttempt() {
        connection?.cancel()
        connection = nil
        stopConnectionTimeout()
        stopHeartbeatLoop()
        connected = false
        authenticated = false
        lastHeartbeatAck = nil
        lastHeartbeatRttMs = nil
        lastHeartbeatSent = nil
        pendingHeartbeatToken = nil
        connectionGeneration &+= 1
        connectionDebugGeneration = connectionGeneration
        connectionAttemptCount &+= 1
        if connectionAttemptCount > 1 {
            reconnectAttemptCount &+= 1
        }
        connectStartedAt = Date()
        lastError = nil
        startConnectionTimeout(for: connectionGeneration)
    }

    private func startConnectionTimeout(for generation: UInt64) {
        stopConnectionTimeout()
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + Self.connectionTimeout)
        timer.setEventHandler { [weak self] in
            Task { @MainActor in
                guard let self, self.connectionGeneration == generation, !self.connected else { return }
                self.protocolError("Connection timed out")
            }
        }
        connectionTimeoutTimer = timer
        timer.resume()
    }

    private func stopConnectionTimeout() {
        connectionTimeoutTimer?.cancel()
        connectionTimeoutTimer = nil
    }

    private func installStateHandler(generation: UInt64) {
        connection?.stateUpdateHandler = { [weak self] state in
            Task { @MainActor in
                guard let self, self.connectionGeneration == generation else { return }
                print("[boo-ios] connection state = \(state)")
                switch state {
                case .ready:
                    self.stopConnectionTimeout()
                    self.connected = true
                    self.lastError = nil
                    if let connectStartedAt = self.connectStartedAt {
                        self.lastConnectLatencyMs = Date().timeIntervalSince(connectStartedAt) * 1000
                        self.connectStartedAt = nil
                    }
                    self.readHeader(generation: generation)
                    self.sendAuth()
                case .waiting(let error):
                    self.connected = false
                    if self.isConnectionRefused(error) {
                        self.stopConnectionTimeout()
                        self.protocolError("Connection refused")
                    } else {
                        self.lastError = self.userFacingTransportMessage(for: error, prefix: "Waiting for network")
                    }
                case .failed(let error):
                    self.stopConnectionTimeout()
                    self.protocolError(self.userFacingTransportMessage(for: error, prefix: "Connection failed"))
                case .cancelled:
                    self.stopConnectionTimeout()
                    self.stopHeartbeatLoop()
                    self.connected = false
                default:
                    break
                }
            }
        }
    }

    private func isConnectionRefused(_ error: NWError) -> Bool {
        if case .posix(let code) = error {
            return code == .ECONNREFUSED
        }
        return false
    }

    private func userFacingTransportMessage(for error: NWError, prefix: String) -> String {
        switch error {
        case .posix(let code):
            switch code {
            case .ENETDOWN:
                return "Network unavailable on this iPad"
            case .ENETUNREACH:
                return "Host network unreachable from this iPad"
            case .EHOSTUNREACH:
                return "Host unreachable from this iPad"
            case .ETIMEDOUT:
                return "Connection timed out"
            case .ECONNRESET:
                return "Connection was reset"
            case .ECONNABORTED:
                return "Connection was interrupted"
            case .ENOTCONN:
                return "Connection is no longer active"
            default:
                return "\(prefix): \(String(describing: code))"
            }
        case .dns(let code):
            return "\(prefix): DNS error (\(code))"
        case .tls(let status):
            return "\(prefix): TLS error (\(status))"
        case .wifiAware(let code):
            return "\(prefix): Wi-Fi aware error (\(code))"
        @unknown default:
            return "\(prefix)"
        }
    }

    private func readHeader(generation: UInt64) {
        connection?.receive(minimumIncompleteLength: Self.headerLen, maximumLength: Self.headerLen) { [weak self] content, _, isComplete, error in
            Task { @MainActor in
                guard let self, self.connectionGeneration == generation else { return }
                if let error {
                    print("[boo-ios] readHeader error = \(error)")
                    self.protocolError("Receive failed: \(error)")
                    return
                }
                guard let data = content, data.count == Self.headerLen else {
                    if isComplete { self.protocolError("Connection closed") }
                    return
                }
                guard data[0] == Self.magic[0], data[1] == Self.magic[1] else {
                    self.lastError = "Invalid protocol header"
                    self.disconnect()
                    return
                }
                let type = data[2]
                let payloadLen = data.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 3, as: UInt32.self)) }
                if payloadLen == 0 {
                    self.handleMessage(type: type, payload: Data())
                    self.readHeader(generation: generation)
                } else {
                    self.readPayload(type: type, length: Int(payloadLen), generation: generation)
                }
            }
        }
    }

    private func readPayload(type: UInt8, length: Int, generation: UInt64) {
        connection?.receive(minimumIncompleteLength: length, maximumLength: length) { [weak self] content, _, isComplete, error in
            Task { @MainActor in
                guard let self, self.connectionGeneration == generation else { return }
                if let error {
                    print("[boo-ios] readPayload error = \(error)")
                    self.protocolError("Receive failed: \(error)")
                    return
                }
                guard let data = content else {
                    if isComplete { self.protocolError("Connection closed") }
                    return
                }
                self.handleMessage(type: type, payload: data)
                self.readHeader(generation: generation)
            }
        }
    }

    private func handleMessage(type: UInt8, payload: Data) {
        guard let message = GSPMessageType(rawValue: type) else { return }
        debugLog("recv \(message)")
        switch message {
        case .authOk:
            if let authRequestedAt {
                lastAuthLatencyMs = Date().timeIntervalSince(authRequestedAt) * 1000
                self.authRequestedAt = nil
            }
            if let error = validateAuthOkPayload(payload) {
                protocolError(error)
                return
            }
            applyReducedMessage(.authOk, payload: payload)
        case .authFail:
            applyReducedMessage(.authFail, payload: payload)
        case .tabList:
            if let tabListRequestedAt {
                lastTabListLatencyMs = Date().timeIntervalSince(tabListRequestedAt) * 1000
                self.tabListRequestedAt = nil
            }
            applyReducedMessage(.tabList, payload: payload)
        case .fullState:
            applyReducedMessage(.fullState, payload: payload)
        case .delta:
            applyReducedMessage(.delta, payload: payload)
        case .tabExited:
            applyReducedMessage(.tabExited, payload: payload)
        case .errorMsg:
            applyReducedMessage(.errorMsg, payload: payload)
        case .heartbeatAck:
            lastHeartbeatAck = Date()
            if payload.count >= 8 {
                let token = payload.withUnsafeBytes {
                    UInt64(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt64.self))
                }
                if let pendingHeartbeatToken, token == pendingHeartbeatToken, let lastHeartbeatSent {
                    lastHeartbeatRttMs = Date().timeIntervalSince(lastHeartbeatSent) * 1000
                    heartbeatAckCount &+= 1
                    self.pendingHeartbeatToken = nil
                }
            }
        case .clipboard:
            handleClipboard(payload)
        case .uiRuntimeState:
            guard let decodedRuntimeState = decodeRemoteRuntimeState(payload) else { return }
            runtimeState = decodedRuntimeState
            activeTabId = runtimeActiveTabId
            if activeTabId != nil {
                pendingHostTabCreation = false
            }
            if authenticated {
                bootstrapCanonicalHostTab(trigger: "runtimeState")
            }
        case .uiAppearance:
            break
        default:
            break
        }
    }

    private func validateAuthOkPayload(_ payload: Data) -> String? {
        validateAuthOkMetadata(payload)
    }

    private var shouldPreserveRemoteStateOnReconnect: Bool {
        !tabs.isEmpty
    }

    private func protocolError(_ message: String) {
        connection?.cancel()
        connection = nil
        stopConnectionTimeout()
        connectionGeneration &+= 1
        connected = false
        authenticated = false
        lastHeartbeatAck = nil
        lastHeartbeatRttMs = nil
        lastHeartbeatSent = nil
        pendingHeartbeatToken = nil
        connectStartedAt = nil
        authRequestedAt = nil
        tabListRequestedAt = nil
        if !shouldPreserveRemoteStateOnReconnect {
            protocolVersion = nil
            transportCapabilities = 0
            serverBuildId = nil
            serverInstanceId = nil
            serverIdentityId = nil
            activeTabId = nil
            tabs = []
            runtimeState = nil
            screen = ScreenState()
        }
        if message == "Remote heartbeat timed out" {
            heartbeatTimeoutCount &+= 1
        }
        lastError = message
        lastErrorKind = .remote(message)
        stopHeartbeatLoop()
    }

    private func applyDecodedTabs(_ decodedTabs: [DecodedWireTabInfo]) {
        tabs = decodedTabs.map {
            RemoteTabInfo(
                id: $0.id,
                name: $0.name,
                title: $0.title,
                pwd: $0.pwd,
                childExited: $0.childExited
            )
        }
    }

    private func applyDecodedScreen(_ decoded: DecodedWireScreenState) {
        screen.rows = decoded.rows
        screen.cols = decoded.cols
        screen.cells = decoded.cells.map {
            WireCell(
                codepoint: $0.codepoint,
                fg_r: $0.fg_r,
                fg_g: $0.fg_g,
                fg_b: $0.fg_b,
                bg_r: $0.bg_r,
                bg_g: $0.bg_g,
                bg_b: $0.bg_b,
                styleFlags: $0.styleFlags,
                wide: $0.wide
            )
        }
        screen.cursorX = decoded.cursorX
        screen.cursorY = decoded.cursorY
        screen.cursorVisible = decoded.cursorVisible
        screen.cursorBlinking = decoded.cursorBlinking
        screen.cursorStyle = decoded.cursorStyle
    }

    private func bootstrapCanonicalHostTab(trigger: String) {
        guard authenticated else { return }
        guard activeTabId == nil else { return }
        guard !autoTabBootstrapSuppressed else { return }

        if let runtimeState,
           runtimeState.tabs.indices.contains(runtimeState.activeTab) {
            let runtimeActiveTabId = runtimeState.tabs[runtimeState.activeTab].tabId
            debugLog("bootstrapCanonicalHostTab trigger=\(trigger) wait runtime active tabId=\(runtimeActiveTabId)")
            return
        }

        debugLog("bootstrapCanonicalHostTab trigger=\(trigger) create")
        createTab()
    }

    private func applyReducedMessage(_ message: ClientWireMessageType, payload: Data) {
        var state = ClientWireState(
            authenticated: authenticated,
            protocolVersion: protocolVersion,
            transportCapabilities: transportCapabilities,
            serverBuildId: serverBuildId,
            serverInstanceId: serverInstanceId,
            serverIdentityId: serverIdentityId,
            tabs: tabs.map {
                DecodedWireTabInfo(
                    id: $0.id,
                    name: $0.name,
                    title: $0.title,
                    pwd: $0.pwd,
                    childExited: $0.childExited
                )
            },
            screen: DecodedWireScreenState(
                rows: screen.rows,
                cols: screen.cols,
                cells: screen.cells.map {
                    DecodedWireCell(
                        codepoint: $0.codepoint,
                        fg_r: $0.fg_r,
                        fg_g: $0.fg_g,
                        fg_b: $0.fg_b,
                        bg_r: $0.bg_r,
                        bg_g: $0.bg_g,
                        bg_b: $0.bg_b,
                        styleFlags: $0.styleFlags,
                        wide: $0.wide
                    )
                },
                cursorX: screen.cursorX,
                cursorY: screen.cursorY,
                cursorVisible: screen.cursorVisible,
                cursorBlinking: screen.cursorBlinking,
                cursorStyle: screen.cursorStyle
            ),
            activeTabId: activeTabId,
            lastErrorKind: lastErrorKind,
            lastError: lastError
        )
        let wasAuthenticated = authenticated
        let effect = ClientWireReducer.reduce(message: message, payload: payload, state: &state)
        authenticated = state.authenticated
        if state.authenticated && !wasAuthenticated {
            startHeartbeatLoop()
        }
        protocolVersion = state.protocolVersion
        transportCapabilities = state.transportCapabilities
        serverBuildId = state.serverBuildId
        serverInstanceId = state.serverInstanceId
        serverIdentityId = state.serverIdentityId
        if let serverInstanceId = state.serverInstanceId, !serverInstanceId.isEmpty {
            lastSeenServerInstanceId = serverInstanceId
        }
        if let serverIdentityId = state.serverIdentityId, !serverIdentityId.isEmpty {
            lastSeenServerIdentityId = serverIdentityId
        }
        lastErrorKind = state.lastErrorKind
        lastError = state.lastError
        let fallbackActiveTabId = state.activeTabId
        let nextActiveTabId = runtimeActiveTabId ?? fallbackActiveTabId
        if nextActiveTabId == nil {
            switch message {
            case .tabExited:
                pendingHostTabCreation = false
            case .errorMsg:
                pendingHostTabCreation = false
            default:
                break
            }
        }
        applyDecodedTabs(state.tabs)
        activeTabId = nextActiveTabId
        if activeTabId != nil {
            pendingHostTabCreation = false
        }
        if let decodedScreen = state.screen {
            applyDecodedScreen(decodedScreen)
            screen.objectWillChange.send()
        }
        if message == .authOk,
           serverIdentityMismatch(
                expectedIdentityId: expectedServerIdentityId,
                actualIdentityId: serverIdentityId
           ) {
            protocolError("Server identity changed; connection rejected")
            return
        }

        switch effect {
        case .none:
            break
        case .listTabs:
            listTabs()
        }

        if message == .authOk {
            if serverIdentityMismatch(
                expectedIdentityId: expectedServerIdentityId,
                actualIdentityId: serverIdentityId
            ) {
                lastError = "Server identity changed; refusing automatic resume"
                return
            }
            return
        }

        if message == .tabList, runtimeState == nil {
            bootstrapCanonicalHostTab(trigger: "tabList")
            return
        }

        if message == .errorMsg {
            switch lastErrorKind {
            case .unknownTab, .noActiveTab:
                pendingHostTabCreation = false
                bootstrapCanonicalHostTab(trigger: "errorRecovery")
            default:
                break
            }
        }
    }

    private func handleClipboard(_ data: Data) {
        guard let encoded = String(data: data, encoding: .utf8),
              let bytes = Data(base64Encoded: encoded),
              let string = String(data: bytes, encoding: .utf8) else { return }
        UIPasteboard.general.string = string
    }
}
