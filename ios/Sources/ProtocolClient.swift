import Foundation
import Network
import Security
import UIKit

enum GSPMessageType: UInt8 {
    case auth = 0x01
    case listTabs = 0x02
    case input = 0x06
    case resize = 0x07
    case scroll = 0x0a
    case appMouseEvent = 0x10
    case heartbeat = 0x11
    case runtimeAction = 0x12
    case renderAck = 0x13

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
    case uiPaneFullState = 0x90
    case uiPaneDelta = 0x91
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

private enum OutboundRuntimeAction: Encodable {
    case setViewedTab(viewId: UInt64, tabId: UInt32)
    case focusPane(viewId: UInt64, tabId: UInt32, paneId: UInt64)
    case newTab(viewId: UInt64, cols: UInt16?, rows: UInt16?)
    case newSplit(viewId: UInt64, direction: String)
    case closeTab(viewId: UInt64, tabId: UInt32?)
    case nextTab(viewId: UInt64)
    case prevTab(viewId: UInt64)
    case attachView(viewId: UInt64)
    case detachView(viewId: UInt64)
    case resizeSplit(viewId: UInt64, direction: String, amount: UInt16, ratio: Double?)
    case noop(viewId: UInt64)

    private enum CodingKeys: String, CodingKey {
        case kind
        case viewId = "view_id"
        case tabId = "tab_id"
        case paneId = "pane_id"
        case cols
        case rows
        case direction
        case amount
        case ratio
    }

    private enum Kind: String, Encodable {
        case setViewedTab = "set_viewed_tab"
        case focusPane = "focus_pane"
        case newTab = "new_tab"
        case newSplit = "new_split"
        case closeTab = "close_tab"
        case nextTab = "next_tab"
        case prevTab = "prev_tab"
        case attachView = "attach_view"
        case detachView = "detach_view"
        case resizeSplit = "resize_split"
        case noop = "noop"
    }

    var traceAction: String {
        switch self {
        case .setViewedTab: return Kind.setViewedTab.rawValue
        case .focusPane: return Kind.focusPane.rawValue
        case .newTab: return Kind.newTab.rawValue
        case .newSplit: return Kind.newSplit.rawValue
        case .closeTab: return Kind.closeTab.rawValue
        case .nextTab: return Kind.nextTab.rawValue
        case .prevTab: return Kind.prevTab.rawValue
        case .attachView: return Kind.attachView.rawValue
        case .detachView: return Kind.detachView.rawValue
        case .resizeSplit: return Kind.resizeSplit.rawValue
        case .noop: return Kind.noop.rawValue
        }
    }

    var traceViewId: UInt64 {
        switch self {
        case .setViewedTab(let viewId, _),
             .focusPane(let viewId, _, _),
             .newTab(let viewId, _, _),
             .newSplit(let viewId, _),
             .closeTab(let viewId, _),
             .nextTab(let viewId),
             .prevTab(let viewId),
             .attachView(let viewId),
             .detachView(let viewId),
             .resizeSplit(let viewId, _, _, _),
             .noop(let viewId):
            return viewId
        }
    }

    var traceTabId: UInt32 {
        switch self {
        case .setViewedTab(_, let tabId),
             .focusPane(_, let tabId, _):
            return tabId
        case .closeTab(_, let tabId):
            return tabId ?? 0
        default:
            return 0
        }
    }

    var tracePaneId: UInt64 {
        switch self {
        case .focusPane(_, _, let paneId):
            return paneId
        default:
            return 0
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .setViewedTab(let viewId, let tabId):
            try container.encode(Kind.setViewedTab, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
            try container.encode(tabId, forKey: .tabId)
        case .focusPane(let viewId, let tabId, let paneId):
            try container.encode(Kind.focusPane, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
            try container.encode(tabId, forKey: .tabId)
            try container.encode(paneId, forKey: .paneId)
        case .newTab(let viewId, let cols, let rows):
            try container.encode(Kind.newTab, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
            try container.encodeIfPresent(cols, forKey: .cols)
            try container.encodeIfPresent(rows, forKey: .rows)
        case .newSplit(let viewId, let direction):
            try container.encode(Kind.newSplit, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
            try container.encode(direction, forKey: .direction)
        case .closeTab(let viewId, let tabId):
            try container.encode(Kind.closeTab, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
            try container.encodeIfPresent(tabId, forKey: .tabId)
        case .nextTab(let viewId):
            try container.encode(Kind.nextTab, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
        case .prevTab(let viewId):
            try container.encode(Kind.prevTab, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
        case .attachView(let viewId):
            try container.encode(Kind.attachView, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
        case .detachView(let viewId):
            try container.encode(Kind.detachView, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
        case .resizeSplit(let viewId, let direction, let amount, let ratio):
            try container.encode(Kind.resizeSplit, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
            try container.encode(direction, forKey: .direction)
            try container.encode(amount, forKey: .amount)
            try container.encodeIfPresent(ratio, forKey: .ratio)
        case .noop(let viewId):
            try container.encode(Kind.noop, forKey: .kind)
            try container.encode(viewId, forKey: .viewId)
        }
    }
}

private enum DecodedInboundMessage {
    case raw(GSPMessageType, Data)
    case heartbeatAck(UInt64?)
    case uiRuntimeState(RemoteRuntimeStateSnapshot)
    case uiPaneFullState(DecodedPaneUpdate, DecodedWireScreenState)
    case uiPaneDelta(DecodedPaneUpdate, Data)
}

private struct OutboundRuntimeActionEnvelope: Encodable {
    let clientActionId: UInt64
    let action: OutboundRuntimeAction

    private enum CodingKeys: String, CodingKey {
        case clientActionId = "client_action_id"
        case action
    }
}

private struct PendingOptimisticRuntimeState {
    let appliedAt: Date
    let previousRuntimeState: RemoteRuntimeStateSnapshot?
}

private struct PendingActionAck {
    let action: String
    let startedAt: Date
    let fields: BooTraceFields
    var optimistic: PendingOptimisticRuntimeState?
}

private extension Data {
    mutating func appendLittleEndian<T: FixedWidthInteger>(_ value: T) {
        var littleEndian = value.littleEndian
        Swift.withUnsafeBytes(of: &littleEndian) { bytes in
            append(contentsOf: bytes)
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
    private static var heartbeatTimeout: TimeInterval {
        // XCUITest accessibility snapshots and screenshots can pause the app's
        // main actor for tens of seconds on a physical iPad. Keep production
        // detection tight, but do not let test-only AX stalls masquerade as a
        // remote transport failure while the runtime-view scenario is probing UI.
        if UITestLaunchConfiguration.current()?.traceActions.contains("runtime-view-e2e") == true {
            return 60
        }
        return 12
    }
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
    @Published var paneScreens: [UInt64: DecodedWireScreenState] = [:]
    @Published private(set) var paneRevisions: [UInt64: UInt64] = [:]
    private var paneServerRevisions: [UInt64: UInt64] = [:]
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
    private var connectStartedAt: Date?
    private var authRequestedAt: Date?
    private var tabListRequestedAt: Date?
    private var renderTraceTracker = BooRenderTraceTracker()
    private var nextClientActionIdValue: UInt64 = 0
    private var pendingActionAcks: [UInt64: PendingActionAck] = [:]
    private var noopBaselineSentForViewIds: Set<UInt64> = []

    private func debugLog(_ message: String) {
        BooTrace.debug(message)
    }

    private func nextTraceInteractionId() -> UInt64 {
        renderTraceTracker.nextInteractionId()
    }

    private func nextClientActionId() -> UInt64 {
        nextClientActionIdValue &+= 1
        return nextClientActionIdValue
    }

    private nonisolated static let magic: [UInt8] = [0x47, 0x53]
    private nonisolated static let headerLen = 7

    private var runtimeActiveTabId: UInt32? {
        runtimeState?.viewedTabId
            ?? {
                guard let runtimeState,
                      runtimeState.tabs.indices.contains(runtimeState.activeTab) else {
                    return nil
                }
                return runtimeState.tabs[runtimeState.activeTab].tabId
            }()
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
        let visiblePaneIds = runtimeState?.visiblePanes.map(\.paneId).map(String.init).joined(separator: ",") ?? ""
        let paneFrameSummary = runtimeState?.visiblePanes.map { pane in
            let frame = pane.frame
            return String(
                format: "%llu:%.1f,%.1f,%.1f,%.1f",
                pane.paneId,
                frame.x,
                frame.y,
                frame.width,
                frame.height
            )
        }.joined(separator: ";") ?? ""
        let paneStateIds = paneScreens.keys.sorted().map(String.init).joined(separator: ",")
        let paneRevisionSummary = paneRevisions.keys.sorted().map { "\($0):\(paneRevisions[$0] ?? 0)" }.joined(separator: ",")
        return [
            "connected=\(connected)",
            "authenticated=\(authenticated)",
            "active=\(activeTabId.map(String.init) ?? "nil")",
            "runtimeActive=\(runtimeActiveTabId.map(String.init) ?? "nil")",
            "tabs=[\(tabIds)]",
            "runtimeTabs=\(runtimeState?.tabs.count ?? 0)",
            "visiblePanes=\(runtimeState?.visiblePanes.count ?? 0)",
            "focusedPane=\(runtimeState?.focusedPane.description ?? "nil")",
            "paneStates=\(paneScreens.count)",
            "visiblePaneIds=[\(visiblePaneIds)]",
            "paneFrames=[\(paneFrameSummary)]",
            "paneStateIds=[\(paneStateIds)]",
            "paneRevisions=[\(paneRevisionSummary)]",
            "heartbeatRttMs=\(lastHeartbeatRttMs.map { String(format: "%.1f", $0) } ?? "nil")",
            "lastError=\(lastError ?? "<none>")",
        ].joined(separator: " ")
    }

    var runtimeAccessibilityTextSnapshot: String {
        guard let runtimeState, !runtimeState.visiblePanes.isEmpty else {
            return screen.accessibilityTextSnapshot
        }
        return runtimeState.visiblePanes.map { pane in
            let text = paneAccessibilityText(paneId: pane.paneId)
            return "pane \(pane.paneId)\(pane.focused ? " focused" : ""):\n\(text)"
        }.joined(separator: "\n---\n")
    }

    func paneAccessibilityText(paneId: UInt64) -> String {
        guard let state = paneScreens[paneId] else { return "" }
        return WireCodec.screenText(from: state)
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
        tabs = []
        runtimeState = nil
        screen = ScreenState()
        paneScreens = [:]
        paneRevisions = [:]
        paneServerRevisions = [:]
        renderTraceTracker = BooRenderTraceTracker()
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

    func configureTrustedServerIdentity(_ identityId: String?) {
        expectedServerIdentityId = identityId
    }

    func clearErrorState() {
        lastErrorKind = nil
        lastError = nil
    }

    func sendInput(_ text: String) {
        guard let data = text.data(using: .utf8) else { return }
        beginInputTrace()
        sendMessage(type: .input, payload: data)
    }

    func sendInputBytes(_ data: Data) {
        beginInputTrace()
        sendMessage(type: .input, payload: data)
    }

    private func beginInputTrace() {
        let state = runtimeState
        renderTraceTracker.beginInput(BooTraceFields(
            interactionId: nextTraceInteractionId(),
            viewId: currentViewId,
            tabId: state?.viewedTabId ?? 0,
            paneId: state?.focusedPane ?? 0,
            action: "input",
            route: "remote",
            runtimeRevision: state?.runtimeRevision ?? 0,
            viewRevision: state?.viewRevision ?? 0,
            paneRevision: 0,
            elapsedMs: 0
        ))
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

    private var currentViewId: UInt64 {
        runtimeState?.viewId ?? 0
    }

    func setViewedTab(_ tabId: UInt32) {
        guard currentViewId != 0 else { return }
        renderTraceTracker.beginRuntimeAction(.remoteSetViewedTab, BooTraceFields(
            interactionId: nextTraceInteractionId(),
            viewId: currentViewId,
            tabId: tabId,
            paneId: 0,
            action: "set_viewed_tab",
            route: "remote",
            runtimeRevision: runtimeState?.runtimeRevision ?? 0,
            viewRevision: runtimeState?.viewRevision ?? 0,
            paneRevision: 0,
            elapsedMs: 0
        ))
        if let clientActionId = sendRuntimeAction(.setViewedTab(viewId: currentViewId, tabId: tabId)) {
            applyOptimisticViewedTab(clientActionId: clientActionId, tabId: tabId)
        }
    }

    func focusPane(tabId: UInt32, paneId: UInt64) {
        guard currentViewId != 0 else { return }
        renderTraceTracker.beginFocusPane(BooTraceFields(
            interactionId: nextTraceInteractionId(),
            viewId: currentViewId,
            tabId: tabId,
            paneId: paneId,
            action: "focus_pane",
            route: "remote",
            runtimeRevision: runtimeState?.runtimeRevision ?? 0,
            viewRevision: runtimeState?.viewRevision ?? 0,
            paneRevision: paneRevisions[paneId] ?? 0,
            elapsedMs: 0
        ))
        if let clientActionId = sendRuntimeAction(.focusPane(viewId: currentViewId, tabId: tabId, paneId: paneId)) {
            applyOptimisticFocusPane(clientActionId: clientActionId, tabId: tabId, paneId: paneId)
        }
    }

    func newTab(cols: UInt16? = nil, rows: UInt16? = nil) {
        guard currentViewId != 0 else { return }
        sendRuntimeAction(.newTab(viewId: currentViewId, cols: cols, rows: rows))
    }

    func newSplit(direction: String = "right") {
        guard currentViewId != 0 else { return }
        sendRuntimeAction(.newSplit(viewId: currentViewId, direction: direction))
    }

    func closeViewedTab() {
        guard currentViewId != 0 else { return }
        sendRuntimeAction(.closeTab(viewId: currentViewId, tabId: runtimeState?.viewedTabId))
    }

    func nextTab() {
        guard currentViewId != 0 else { return }
        sendRuntimeAction(.nextTab(viewId: currentViewId))
    }

    func prevTab() {
        guard currentViewId != 0 else { return }
        sendRuntimeAction(.prevTab(viewId: currentViewId))
    }

    func resizeSplit(
        direction: String,
        ratio: Double,
        optimisticFirstPaneId: UInt64? = nil,
        optimisticSecondPaneId: UInt64? = nil
    ) {
        guard currentViewId != 0 else { return }
        let clamped = min(max(ratio, 0.1), 0.9)
        renderTraceTracker.beginRuntimeAction(.remoteResizeSplit, BooTraceFields(
            interactionId: nextTraceInteractionId(),
            viewId: currentViewId,
            tabId: runtimeState?.viewedTabId ?? 0,
            paneId: runtimeState?.focusedPane ?? 0,
            action: "resize_split",
            route: "remote",
            runtimeRevision: runtimeState?.runtimeRevision ?? 0,
            viewRevision: runtimeState?.viewRevision ?? 0,
            paneRevision: 0,
            elapsedMs: 0
        ))
        let clientActionId = sendRuntimeAction(
            .resizeSplit(
                viewId: currentViewId,
                direction: direction,
                amount: 0,
                ratio: clamped
            )
        )
        if let clientActionId,
           let optimisticFirstPaneId,
           let optimisticSecondPaneId
        {
            applyOptimisticResizeSplit(
                clientActionId: clientActionId,
                direction: direction,
                ratio: clamped,
                firstPaneId: optimisticFirstPaneId,
                secondPaneId: optimisticSecondPaneId
            )
        }
    }

    func attachView() {
        guard currentViewId != 0 else { return }
        sendRuntimeAction(.attachView(viewId: currentViewId))
    }

    func detachView() {
        guard currentViewId != 0 else { return }
        sendRuntimeAction(.detachView(viewId: currentViewId))
    }

    private func sendNoopBaselineIfNeeded(for state: RemoteRuntimeStateSnapshot) {
        guard state.viewId != 0, !noopBaselineSentForViewIds.contains(state.viewId) else { return }
        noopBaselineSentForViewIds.insert(state.viewId)
        sendRuntimeAction(.noop(viewId: state.viewId))
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

    @discardableResult
    private func sendRuntimeAction(_ action: OutboundRuntimeAction) -> UInt64? {
        let clientActionId = nextClientActionId()
        let fields = BooTraceFields(
            interactionId: clientActionId,
            viewId: action.traceViewId,
            tabId: action.traceTabId,
            paneId: action.tracePaneId,
            action: action.traceAction,
            route: "remote",
            runtimeRevision: runtimeState?.runtimeRevision ?? 0,
            viewRevision: runtimeState?.viewRevision ?? 0,
            paneRevision: 0,
            elapsedMs: 0
        )
        pendingActionAcks[clientActionId] = PendingActionAck(
            action: action.traceAction,
            startedAt: Date(),
            fields: fields,
            optimistic: nil
        )
        guard let payload = try? JSONEncoder().encode(
            OutboundRuntimeActionEnvelope(clientActionId: clientActionId, action: action)
        ) else {
            pendingActionAcks.removeValue(forKey: clientActionId)
            return nil
        }
        BooTrace.log(.remoteRuntimeAction, fields)
        sendMessage(type: .runtimeAction, payload: payload)
        return clientActionId
    }

    private func applyOptimisticFocusPane(clientActionId: UInt64, tabId: UInt32, paneId: UInt64) {
        guard let currentState = runtimeState,
              currentState.viewedTabId == tabId,
              currentState.visiblePaneIds.contains(paneId),
              currentState.focusedPane != paneId
        else { return }

        runtimeState = currentState.withOptimisticFocus(tabId: tabId, paneId: paneId)
        activeTabId = runtimeActiveTabId
        markOptimisticRuntimeState(clientActionId: clientActionId, previousState: currentState, paneId: paneId)
    }

    private func applyOptimisticViewedTab(clientActionId: UInt64, tabId: UInt32) {
        guard let currentState = runtimeState,
              currentState.tabs.contains(where: { $0.tabId == tabId }),
              currentState.viewedTabId != tabId
        else { return }

        runtimeState = currentState.withOptimisticViewedTab(tabId)
        activeTabId = runtimeActiveTabId
        markOptimisticRuntimeState(clientActionId: clientActionId, previousState: currentState, paneId: 0)
    }

    private func applyOptimisticResizeSplit(
        clientActionId: UInt64,
        direction: String,
        ratio: Double,
        firstPaneId: UInt64,
        secondPaneId: UInt64
    ) {
        guard let currentState = runtimeState,
              currentState.visiblePaneIds.contains(firstPaneId),
              currentState.visiblePaneIds.contains(secondPaneId)
        else { return }

        runtimeState = currentState.withOptimisticResize(
            direction: direction,
            ratio: ratio,
            firstPaneId: firstPaneId,
            secondPaneId: secondPaneId
        )
        activeTabId = runtimeActiveTabId
        markOptimisticRuntimeState(
            clientActionId: clientActionId,
            previousState: currentState,
            paneId: currentState.focusedPane
        )
    }

    private func markOptimisticRuntimeState(
        clientActionId: UInt64,
        previousState: RemoteRuntimeStateSnapshot,
        paneId: UInt64
    ) {
        guard var pending = pendingActionAcks[clientActionId] else { return }
        pending.optimistic = PendingOptimisticRuntimeState(
            appliedAt: Date(),
            previousRuntimeState: previousState
        )
        pendingActionAcks[clientActionId] = pending

        var fields = pending.fields
        fields.paneId = paneId
        fields.elapsedMs = 0
        BooTrace.log(.remoteOptimisticApply, fields)
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
        pendingActionAcks.removeAll()
        noopBaselineSentForViewIds.removeAll()
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
                BooTrace.debug("connection state = \(state)")
                switch state {
                case .ready:
                    self.stopConnectionTimeout()
                    self.connected = true
                    self.lastError = nil
                    if let connectStartedAt = self.connectStartedAt {
                        self.lastConnectLatencyMs = Date().timeIntervalSince(connectStartedAt) * 1000
                        BooTrace.log(.remoteConnect, BooTraceFields(
                            interactionId: self.connectionGeneration,
                            viewId: self.currentViewId,
                            tabId: self.runtimeState?.viewedTabId ?? 0,
                            paneId: self.runtimeState?.focusedPane ?? 0,
                            action: "connect",
                            route: "quic",
                            runtimeRevision: self.runtimeState?.runtimeRevision ?? 0,
                            viewRevision: self.runtimeState?.viewRevision ?? 0,
                            paneRevision: 0,
                            elapsedMs: self.lastConnectLatencyMs ?? 0
                        ))
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
        @unknown default:
            return "\(prefix)"
        }
    }

    private enum HeaderDecodeResult {
        case failure(String)
        case closed
        case incomplete
        case invalidMagic
        case frame(type: UInt8, payloadLen: UInt32)
    }

    private enum PayloadDecodeResult {
        case failure(String)
        case closed
        case incomplete
        case message(DecodedInboundMessage)
    }

    nonisolated private static func decodeHeader(
        content: Data?,
        isComplete: Bool,
        error: NWError?
    ) -> HeaderDecodeResult {
        if let error {
            BooTrace.error("readHeader error = \(error)")
            return .failure("Receive failed: \(error)")
        }
        guard let data = content, data.count == headerLen else {
            return isComplete ? .closed : .incomplete
        }
        guard data[0] == magic[0], data[1] == magic[1] else {
            return .invalidMagic
        }
        let payloadLen = data.withUnsafeBytes {
            UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 3, as: UInt32.self))
        }
        return .frame(type: data[2], payloadLen: payloadLen)
    }

    nonisolated private static func decodePayload(
        type: UInt8,
        content: Data?,
        isComplete: Bool,
        error: NWError?
    ) -> PayloadDecodeResult {
        if let error {
            BooTrace.error("readPayload error = \(error)")
            return .failure("Receive failed: \(error)")
        }
        guard let data = content else {
            return isComplete ? .closed : .incomplete
        }
        return .message(decodeInboundMessage(type: type, payload: data))
    }

    nonisolated private static func decodeInboundMessage(
        type: UInt8,
        payload: Data
    ) -> DecodedInboundMessage {
        guard let message = GSPMessageType(rawValue: type) else {
            return .raw(.errorMsg, Data())
        }
        switch message {
        case .heartbeatAck:
            let token = payload.count >= 8
                ? payload.withUnsafeBytes {
                    UInt64(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt64.self))
                }
                : nil
            return .heartbeatAck(token)
        case .uiRuntimeState:
            if let state = decodeRemoteRuntimeState(payload) {
                return .uiRuntimeState(state)
            }
            return .raw(message, payload)
        case .uiPaneFullState:
            if let (update, state) = WireCodec.decodePaneFullState(payload) {
                return .uiPaneFullState(update, state)
            }
            return .raw(message, payload)
        case .uiPaneDelta:
            if let (update, deltaPayload) = WireCodec.decodePaneDelta(payload) {
                return .uiPaneDelta(update, deltaPayload)
            }
            return .raw(message, payload)
        default:
            return .raw(message, payload)
        }
    }

    private func readHeader(generation: UInt64) {
        connection?.receive(minimumIncompleteLength: Self.headerLen, maximumLength: Self.headerLen) { [weak self] content, _, isComplete, error in
            let headerResult = Self.decodeHeader(content: content, isComplete: isComplete, error: error)
            Task { @MainActor in
                guard let self, self.connectionGeneration == generation else { return }
                switch headerResult {
                case .failure(let message):
                    self.protocolError(message)
                case .closed:
                    self.protocolError("Connection closed")
                case .incomplete:
                    return
                case .invalidMagic:
                    self.lastError = "Invalid protocol header"
                    self.disconnect()
                case .frame(let type, let payloadLen):
                    if payloadLen == 0 {
                        self.handleDecodedMessage(Self.decodeInboundMessage(type: type, payload: Data()))
                        self.readHeader(generation: generation)
                    } else {
                        self.readPayload(type: type, length: Int(payloadLen), generation: generation)
                    }
                }
            }
        }
    }

    private func readPayload(type: UInt8, length: Int, generation: UInt64) {
        connection?.receive(minimumIncompleteLength: length, maximumLength: length) { [weak self] content, _, isComplete, error in
            let payloadResult = Self.decodePayload(type: type, content: content, isComplete: isComplete, error: error)
            Task { @MainActor in
                guard let self, self.connectionGeneration == generation else { return }
                switch payloadResult {
                case .failure(let message):
                    self.protocolError(message)
                case .closed:
                    self.protocolError("Connection closed")
                case .incomplete:
                    return
                case .message(let decoded):
                    self.handleDecodedMessage(decoded)
                    self.readHeader(generation: generation)
                }
            }
        }
    }

    private func handleMessage(type: UInt8, payload: Data) {
        guard let message = GSPMessageType(rawValue: type) else { return }
        handleDecodedMessage(Self.decodeInboundMessage(type: message.rawValue, payload: payload))
    }

    private func handleDecodedMessage(_ decoded: DecodedInboundMessage) {
        switch decoded {
        case .raw(let message, let payload):
            handleRawMessage(message, payload: payload)
        case .heartbeatAck(let token):
            handleHeartbeatAck(token: token)
        case .uiRuntimeState(let decodedRuntimeState):
            runtimeState = decodedRuntimeState
            completeActionAckIfNeeded(decodedRuntimeState)
            let visible = Set(decodedRuntimeState.visiblePaneIds)
            paneScreens = paneScreens.filter { visible.contains($0.key) }
            paneRevisions = paneRevisions.filter { visible.contains($0.key) }
            paneServerRevisions = paneServerRevisions.filter { visible.contains($0.key) }
            activeTabId = runtimeActiveTabId
            sendNoopBaselineIfNeeded(for: decodedRuntimeState)
        case .uiPaneFullState(let update, let state):
            applyPaneFullState(update: update, state: state)
        case .uiPaneDelta(let update, let deltaPayload):
            applyPaneDelta(update: update, deltaPayload: deltaPayload)
        }
    }

    private func handleRawMessage(_ message: GSPMessageType, payload: Data) {
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
            completeFocusedScreenRenderTraceIfNeeded()
        case .delta:
            applyReducedMessage(.delta, payload: payload)
            completeFocusedScreenRenderTraceIfNeeded()
        case .tabExited:
            applyReducedMessage(.tabExited, payload: payload)
        case .errorMsg:
            applyReducedMessage(.errorMsg, payload: payload)
        case .heartbeatAck:
            handleHeartbeatAck(token: payload.count >= 8 ? payload.withUnsafeBytes {
                UInt64(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt64.self))
            } : nil)
        case .clipboard:
            handleClipboard(payload)
        case .uiRuntimeState:
            break
        case .uiAppearance:
            break
        case .uiPaneFullState:
            break
        case .uiPaneDelta:
            break
        default:
            break
        }
    }

    private func bumpPaneRenderRevision(_ paneId: UInt64) {
        var updated = paneRevisions
        updated[paneId] = (updated[paneId] ?? 0) &+ 1
        paneRevisions = updated
    }

    private func handleHeartbeatAck(token: UInt64?) {
        lastHeartbeatAck = Date()
        guard let token,
              let pendingHeartbeatToken,
              token == pendingHeartbeatToken,
              let lastHeartbeatSent
        else { return }
        let rttMs = Date().timeIntervalSince(lastHeartbeatSent) * 1000
        lastHeartbeatRttMs = rttMs
        heartbeatAckCount &+= 1
        self.pendingHeartbeatToken = nil
        BooTrace.log(.remoteHeartbeatRtt, BooTraceFields(
            interactionId: token,
            viewId: currentViewId,
            tabId: runtimeState?.viewedTabId ?? 0,
            paneId: runtimeState?.focusedPane ?? 0,
            action: "heartbeat_ack",
            route: "remote",
            runtimeRevision: runtimeState?.runtimeRevision ?? 0,
            viewRevision: runtimeState?.viewRevision ?? 0,
            paneRevision: runtimeState.map { paneRevisions[$0.focusedPane] ?? 0 } ?? 0,
            elapsedMs: rttMs
        ))
    }

    private func applyPaneFullState(update: DecodedPaneUpdate, state: DecodedWireScreenState) {
        guard paneUpdateTargetsCurrentView(update) else { return }
        let lastRevision = paneServerRevisions[update.paneId] ?? 0
        guard update.paneRevision >= lastRevision else { return }
        paneServerRevisions[update.paneId] = update.paneRevision
        setPaneScreen(update.paneId, state)
        bumpPaneRenderRevision(update.paneId)
        tracePaneUpdate(update, action: "pane_update")
        if paneUpdateIsFocusedVisiblePane(update) {
            applyDecodedScreen(state)
            screen.objectWillChange.send()
            completeRenderTraceIfNeeded(update: update)
        }
    }

    private func applyPaneDelta(update: DecodedPaneUpdate, deltaPayload: Data) {
        guard paneUpdateTargetsCurrentView(update) else { return }
        let lastRevision = paneServerRevisions[update.paneId] ?? 0
        guard update.paneRevision >= lastRevision else { return }
        guard var state = paneScreens[update.paneId] else { return }
        guard WireCodec.applyPaneDelta(deltaPayload, to: &state) else { return }
        paneServerRevisions[update.paneId] = update.paneRevision
        setPaneScreen(update.paneId, state)
        bumpPaneRenderRevision(update.paneId)
        tracePaneUpdate(update, action: "pane_update")
        if paneUpdateIsFocusedVisiblePane(update) {
            applyDecodedScreen(state)
            screen.objectWillChange.send()
            completeRenderTraceIfNeeded(update: update)
        }
    }

    private func setPaneScreen(_ paneId: UInt64, _ state: DecodedWireScreenState) {
        var updated = paneScreens
        updated[paneId] = state
        paneScreens = updated
    }

    private func mirrorFocusedLegacyScreen(_ decoded: DecodedWireScreenState) {
        guard let runtimeState,
              runtimeState.visiblePaneIds.contains(runtimeState.focusedPane)
        else { return }
        setPaneScreen(runtimeState.focusedPane, decoded)
        bumpPaneRenderRevision(runtimeState.focusedPane)
    }

    private func tracePaneUpdate(_ update: DecodedPaneUpdate, action: String) {
        BooTrace.log(.remotePaneUpdate, BooTraceFields(
            interactionId: 0,
            viewId: runtimeState?.viewId ?? currentViewId,
            tabId: update.tabId,
            paneId: update.paneId,
            action: action,
            route: "remote",
            runtimeRevision: update.runtimeRevision,
            viewRevision: runtimeState?.viewRevision ?? 0,
            paneRevision: update.paneRevision,
            elapsedMs: 0
        ))
    }

    private func completeActionAckIfNeeded(_ state: RemoteRuntimeStateSnapshot) {
        guard let clientActionId = state.ackedClientActionId,
              let pending = pendingActionAcks.removeValue(forKey: clientActionId)
        else { return }
        let elapsedMs = Date().timeIntervalSince(pending.startedAt) * 1000
        var fields = pending.fields
        fields.viewId = state.viewId
        fields.tabId = state.viewedTabId ?? fields.tabId
        fields.paneId = state.focusedPane
        fields.runtimeRevision = state.runtimeRevision
        fields.viewRevision = state.viewRevision
        fields.elapsedMs = elapsedMs
        BooTrace.log(.remoteActionAck, fields)
        if pending.action == "noop" {
            BooTrace.log(.remoteNoopRoundtrip, fields)
        }
        if let optimistic = pending.optimistic {
            var reconcileFields = fields
            reconcileFields.elapsedMs = Date().timeIntervalSince(optimistic.appliedAt) * 1000
            BooTrace.log(.remoteReconcile, reconcileFields)
        }
    }

    private func paneUpdateIsFocusedVisiblePane(_ update: DecodedPaneUpdate) -> Bool {
        guard let runtimeState else { return false }
        return update.tabId == runtimeState.viewedTabId
            && update.paneId == runtimeState.focusedPane
    }

    private func paneUpdateTargetsCurrentView(_ update: DecodedPaneUpdate) -> Bool {
        guard let runtimeState else { return true }
        return update.tabId == runtimeState.viewedTabId
            && runtimeState.visiblePaneIds.contains(update.paneId)
    }

    private func completeRenderTraceIfNeeded(update: DecodedPaneUpdate) {
        let fields = BooTraceFields(
            interactionId: 0,
            viewId: runtimeState?.viewId ?? currentViewId,
            tabId: update.tabId,
            paneId: update.paneId,
            action: "render_apply",
            route: "remote",
            runtimeRevision: update.runtimeRevision,
            viewRevision: runtimeState?.viewRevision ?? 0,
            paneRevision: update.paneRevision,
            elapsedMs: 0
        )
        renderTraceTracker.completeRenderApply(fields: fields, tabId: update.tabId)
        sendRenderAck(update: update)
    }

    private func sendRenderAck(update: DecodedPaneUpdate) {
        var payload = Data()
        payload.appendLittleEndian(runtimeState?.viewId ?? currentViewId)
        payload.appendLittleEndian(update.tabId)
        payload.appendLittleEndian(update.paneId)
        payload.appendLittleEndian(update.paneRevision)
        payload.appendLittleEndian(update.runtimeRevision)
        sendMessage(type: .renderAck, payload: payload)
        BooTrace.log(.remoteRenderAck, BooTraceFields(
            interactionId: 0,
            viewId: runtimeState?.viewId ?? currentViewId,
            tabId: update.tabId,
            paneId: update.paneId,
            action: "render_ack",
            route: "remote",
            runtimeRevision: update.runtimeRevision,
            viewRevision: runtimeState?.viewRevision ?? 0,
            paneRevision: update.paneRevision,
            elapsedMs: 0
        ))
    }

    private func completeFocusedScreenRenderTraceIfNeeded() {
        guard let state = runtimeState else { return }
        let tabId = state.viewedTabId
        let paneId = state.focusedPane
        let paneUpdateFields = BooTraceFields(
            interactionId: 0,
            viewId: state.viewId,
            tabId: tabId ?? 0,
            paneId: paneId,
            action: "pane_update",
            route: "remote",
            runtimeRevision: state.runtimeRevision,
            viewRevision: state.viewRevision,
            paneRevision: paneRevisions[paneId] ?? 0,
            elapsedMs: 0
        )
        BooTrace.log(.remotePaneUpdate, paneUpdateFields)

        var renderFields = paneUpdateFields
        renderFields.action = "render_apply"
        renderTraceTracker.completeRenderApply(fields: renderFields, tabId: tabId)
    }

    private func validateAuthOkPayload(_ payload: Data) -> String? {
        validateAuthOkMetadata(payload)
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
        protocolVersion = nil
        transportCapabilities = 0
        serverBuildId = nil
        serverInstanceId = nil
        serverIdentityId = nil
        activeTabId = nil
        tabs = []
        runtimeState = nil
        screen = ScreenState()
        paneScreens = [:]
        paneRevisions = [:]
        paneServerRevisions = [:]
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
                active: $0.active,
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
            tabs: tabs.map {
                DecodedWireTabInfo(
                    id: $0.id,
                    name: $0.name,
                    title: $0.title,
                    pwd: $0.pwd,
                    active: $0.active,
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
            lastErrorKind: lastErrorKind,
            lastError: lastError
        )
        let wasAuthenticated = authenticated
        ClientWireReducer.reduce(message: message, payload: payload, state: &state)
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
        applyDecodedTabs(state.tabs)
        activeTabId = runtimeActiveTabId
        if let decodedScreen = state.screen {
            applyDecodedScreen(decodedScreen)
            if message == .fullState || message == .delta {
                mirrorFocusedLegacyScreen(decodedScreen)
            }
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

    }

    private func handleClipboard(_ data: Data) {
        guard let encoded = String(data: data, encoding: .utf8),
              let bytes = Data(base64Encoded: encoded),
              let string = String(data: bytes, encoding: .utf8) else { return }
        UIPasteboard.general.string = string
    }
}
