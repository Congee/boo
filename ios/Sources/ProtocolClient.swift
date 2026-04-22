import Foundation
import Network
import Security
import UIKit

enum GSPMessageType: UInt8 {
    case auth = 0x01
    case listSessions = 0x02
    case attach = 0x03
    case detach = 0x04
    case create = 0x05
    case input = 0x06
    case resize = 0x07
    case destroy = 0x08
    case scroll = 0x0a
    case heartbeat = 0x11

    case authOk = 0x80
    case authFail = 0x81
    case sessionList = 0x82
    case fullState = 0x83
    case delta = 0x84
    case attached = 0x85
    case detached = 0x86
    case errorMsg = 0x87
    case sessionCreated = 0x88
    case sessionExited = 0x89
    case scrollData = 0x8a
    case clipboard = 0x8b
    case image = 0x8c
    case heartbeatAck = 0x92
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
                        let existingScore = existing.advertisedPort == 7337 ? 1 : 0
                        let candidateScore = ranked.advertisedPort == 7337 ? 1 : 0
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
    @Published var lastSessionListLatencyMs: Double?
    @Published var lastAttachLatencyMs: Double?
    @Published var connectionAttemptCount: UInt32 = 0
    @Published var reconnectAttemptCount: UInt32 = 0
    @Published var connectionDebugGeneration: UInt64 = 0
    @Published var sessions: [SessionInfo] = []
    @Published var screen = ScreenState()
    @Published var attachedSessionId: UInt32?
    @Published var attachmentId: UInt64?
    @Published var resumeToken: UInt64?
    @Published var pendingAttachedSessionId: UInt32?
    @Published var lastErrorKind: ClientWireErrorKind?
    @Published var lastError: String?

    private var connection: NWConnection?
    private let queue = DispatchQueue(label: "boo-gsp-client", qos: .userInteractive)
    private var heartbeatTimer: DispatchSourceTimer?
    private var connectionTimeoutTimer: DispatchSourceTimer?
    private var lastHeartbeatSent: Date?
    private var pendingHeartbeatToken: UInt64?
    private var desiredAttachedSessionId: UInt32?
    private var desiredAttachmentId: UInt64?
    private var desiredResumeToken: UInt64?
    private var expectedServerIdentityId: String?
    private var connectionGeneration: UInt64 = 0
    private var connectStartedAt: Date?
    private var authRequestedAt: Date?
    private var sessionListRequestedAt: Date?
    private var attachRequestedAt: Date?

    private nonisolated static let magic: [UInt8] = [0x47, 0x53]
    private nonisolated static let headerLen = 7

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
        let listed = lastSessionListLatencyMs.map { String(format: "list %.0fms", $0) }
        let attached = lastAttachLatencyMs.map { String(format: "att %.0fms", $0) }
        let attachment = attachmentId.map { "attach 0x" + String($0, radix: 16) }
        let resume = resumeToken.map { "resume 0x" + String($0, radix: 16) }
        let base = [ "proto \(protocolVersion)",
                     "caps 0x\(String(transportCapabilities, radix: 16))",
                     "gen \(connectionDebugGeneration)",
                     "conn# \(connectionAttemptCount)",
                     reconnectAttemptCount > 0 ? "reconn# \(reconnectAttemptCount)" : nil,
                     serverBuildId,
                     "id \(serverIdentityId)",
                     "srv \(serverInstanceId)",
                     attachment,
                     resume].compactMap { $0 }.joined(separator: " · ")
        let timings = [connect, auth, listed, attached, heartbeat].compactMap { $0 }
        if !timings.isEmpty {
            return "\(base) · \(timings.joined(separator: " · "))"
        }
        return base
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
        attachedSessionId = nil
        attachmentId = nil
        resumeToken = nil
        pendingAttachedSessionId = nil
        desiredAttachedSessionId = nil
        desiredAttachmentId = nil
        desiredResumeToken = nil
        sessions = []
        screen = ScreenState()
        stopHeartbeatLoop()
        connectStartedAt = nil
        authRequestedAt = nil
        sessionListRequestedAt = nil
        attachRequestedAt = nil
        lastErrorKind = nil
        lastError = nil
    }

    func listSessions() {
        sessionListRequestedAt = Date()
        sendMessage(type: .listSessions, payload: Data())
    }

    func createSession(cols: UInt16 = 120, rows: UInt16 = 36) {
        var payload = Data(count: 4)
        payload.withUnsafeMutableBytes { buf in
            buf.storeBytes(of: cols.littleEndian, as: UInt16.self)
            buf.storeBytes(of: rows.littleEndian, toByteOffset: 2, as: UInt16.self)
        }
        sendMessage(type: .create, payload: payload)
    }

    func destroySession(sessionId: UInt32) {
        var payload = Data(count: 4)
        payload.withUnsafeMutableBytes { buf in
            buf.storeBytes(of: sessionId.littleEndian, as: UInt32.self)
        }
        sendMessage(type: .destroy, payload: payload)
    }

    func attach(sessionId: UInt32) {
        let newAttachmentId = generateAttachmentId()
        desiredAttachedSessionId = sessionId
        desiredAttachmentId = newAttachmentId
        desiredResumeToken = nil
        pendingAttachedSessionId = sessionId
        lastError = nil
        sendAttach(sessionId: sessionId, attachmentId: newAttachmentId, resumeToken: nil)
    }

    func configureResumeAttachment(sessionId: UInt32, attachmentId: UInt64, resumeToken: UInt64) {
        desiredAttachedSessionId = sessionId
        desiredAttachmentId = attachmentId
        desiredResumeToken = resumeToken
        pendingAttachedSessionId = sessionId
    }

    func clearResumeAttachmentState() {
        attachmentId = nil
        resumeToken = nil
        pendingAttachedSessionId = nil
        desiredAttachedSessionId = nil
        desiredAttachmentId = nil
        desiredResumeToken = nil
    }

    func configureTrustedServerIdentity(_ identityId: String?) {
        expectedServerIdentityId = identityId
    }

    func clearErrorState() {
        lastErrorKind = nil
        lastError = nil
    }

    private func sendAttach(sessionId: UInt32, attachmentId: UInt64, resumeToken: UInt64?) {
        attachRequestedAt = Date()
        var payload = Data(count: resumeToken == nil ? 12 : 20)
        payload.withUnsafeMutableBytes { $0.storeBytes(of: sessionId.littleEndian, as: UInt32.self) }
        payload.withUnsafeMutableBytes { $0.storeBytes(of: attachmentId.littleEndian, toByteOffset: 4, as: UInt64.self) }
        if let resumeToken {
            payload.withUnsafeMutableBytes { $0.storeBytes(of: resumeToken.littleEndian, toByteOffset: 12, as: UInt64.self) }
        }
        sendMessage(type: .attach, payload: payload)
    }

    private func generateAttachmentId() -> UInt64 {
        var generator = SystemRandomNumberGenerator()
        var attachmentId = UInt64.random(in: UInt64.min...UInt64.max, using: &generator)
        if attachmentId == 0 {
            attachmentId = 1
        }
        return attachmentId
    }

    func detach() {
        sendMessage(type: .detach, payload: Data())
        attachedSessionId = nil
        attachmentId = nil
        resumeToken = nil
        pendingAttachedSessionId = nil
        desiredAttachedSessionId = nil
        desiredAttachmentId = nil
        desiredResumeToken = nil
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
        case .sessionList:
            if let sessionListRequestedAt {
                lastSessionListLatencyMs = Date().timeIntervalSince(sessionListRequestedAt) * 1000
                self.sessionListRequestedAt = nil
            }
            applyReducedMessage(.sessionList, payload: payload)
        case .attached:
            if let attachRequestedAt {
                lastAttachLatencyMs = Date().timeIntervalSince(attachRequestedAt) * 1000
                self.attachRequestedAt = nil
            }
            applyReducedMessage(.attached, payload: payload)
        case .sessionCreated:
            applyReducedMessage(.sessionCreated, payload: payload)
        case .fullState:
            applyReducedMessage(.fullState, payload: payload)
        case .delta:
            applyReducedMessage(.delta, payload: payload)
        case .detached:
            applyReducedMessage(.detached, payload: payload)
        case .sessionExited:
            applyReducedMessage(.sessionExited, payload: payload)
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
        default:
            break
        }
    }

    private func validateAuthOkPayload(_ payload: Data) -> String? {
        validateAuthOkMetadata(payload)
    }

    private var shouldPreserveRemoteStateOnReconnect: Bool {
        desiredAttachedSessionId != nil || !sessions.isEmpty
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
        sessionListRequestedAt = nil
        attachRequestedAt = nil
        if !shouldPreserveRemoteStateOnReconnect {
            protocolVersion = nil
            transportCapabilities = 0
            serverBuildId = nil
            serverInstanceId = nil
            serverIdentityId = nil
            attachedSessionId = nil
            attachmentId = nil
            resumeToken = nil
            pendingAttachedSessionId = nil
            sessions = []
            screen = ScreenState()
        }
        if message == "Remote heartbeat timed out" {
            heartbeatTimeoutCount &+= 1
        }
        lastError = message
        lastErrorKind = .remote(message)
        stopHeartbeatLoop()
    }

    private func applyDecodedSessions(_ decodedSessions: [DecodedWireSessionInfo]) {
        sessions = decodedSessions.map {
            SessionInfo(
                id: $0.id,
                name: $0.name,
                title: $0.title,
                pwd: $0.pwd,
                attached: $0.attached,
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

    private func applyReducedMessage(_ message: ClientWireMessageType, payload: Data) {
        var state = ClientWireState(
            authenticated: authenticated,
            protocolVersion: protocolVersion,
            transportCapabilities: transportCapabilities,
            serverBuildId: serverBuildId,
            serverInstanceId: serverInstanceId,
            serverIdentityId: serverIdentityId,
            sessions: sessions.map {
                DecodedWireSessionInfo(
                    id: $0.id,
                    name: $0.name,
                    title: $0.title,
                    pwd: $0.pwd,
                    attached: $0.attached,
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
            attachedSessionId: attachedSessionId,
            attachmentId: attachmentId,
            resumeToken: resumeToken,
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
        if state.lastErrorKind?.invalidatesResumeAttachment == true {
            clearResumeAttachmentState()
        }
        attachedSessionId = state.attachedSessionId
        attachmentId = state.attachmentId
        resumeToken = state.resumeToken
        if attachedSessionId != nil {
            pendingAttachedSessionId = nil
        } else {
            switch message {
            case .detached, .sessionExited:
                pendingAttachedSessionId = nil
            case .errorMsg where pendingAttachedSessionId != nil:
                pendingAttachedSessionId = nil
            default:
                break
            }
        }
        applyDecodedSessions(state.sessions)
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
        case .listSessions:
            listSessions()
        case .attach(let sessionId):
            attach(sessionId: sessionId)
        }

        if message == .sessionList,
           let desiredSessionId = desiredAttachedSessionId,
           let desiredAttachmentId,
           let desiredResumeToken,
           attachedSessionId == nil,
           sessions.contains(where: { $0.id == desiredSessionId }) {
            if serverIdentityMismatch(
                expectedIdentityId: expectedServerIdentityId,
                actualIdentityId: serverIdentityId
            ) {
                lastError = "Server identity changed; refusing automatic resume"
                return
            }
            sendAttach(
                sessionId: desiredSessionId,
                attachmentId: desiredAttachmentId,
                resumeToken: desiredResumeToken
            )
        }
    }

    private func handleClipboard(_ data: Data) {
        guard let encoded = String(data: data, encoding: .utf8),
              let bytes = Data(base64Encoded: encoded),
              let string = String(data: bytes, encoding: .utf8) else { return }
        UIPasteboard.general.string = string
    }
}
