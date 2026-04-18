import Foundation
import Network
import CryptoKit
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
    case authChallenge = 0x09
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

struct SessionInfo: Identifiable {
    let id: UInt32
    let name: String
    let title: String
    let pwd: String
    let attached: Bool
    let childExited: Bool
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
}

struct DiscoveredDaemon: Identifiable, Hashable {
    let id: String
    let name: String
    let endpoint: NWEndpoint
}

@MainActor
final class BonjourBrowser: ObservableObject {
    @Published var daemons: [DiscoveredDaemon] = []
    @Published var isSearching = false

    private var browsers: [NWBrowser] = []
    private let queue = DispatchQueue(label: "boo-bonjour-browser")
    private let serviceTypes = ["_boo._tcp"]

    func startBrowsing() {
        stopBrowsing()
        isSearching = true
        for type in serviceTypes {
            let descriptor = NWBrowser.Descriptor.bonjour(type: type, domain: nil)
            let params = NWParameters()
            params.includePeerToPeer = true
            let browser = NWBrowser(for: descriptor, using: params)
            browser.stateUpdateHandler = { [weak self] state in
                Task { @MainActor in
                    if case .failed = state { self?.isSearching = false }
                    if case .cancelled = state { self?.isSearching = false }
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
    }

    private func refreshDiscoveredDaemons() {
        Task { @MainActor in
            var seen = Set<String>()
            var entries: [DiscoveredDaemon] = []
            for browser in browsers {
                for result in browser.browseResults {
                    let id = "\(result.endpoint)"
                    guard seen.insert(id).inserted else { continue }
                    let name: String
                    switch result.endpoint {
                    case .service(let n, _, _, _):
                        name = n
                    default:
                        name = id
                    }
                    entries.append(DiscoveredDaemon(id: id, name: name, endpoint: result.endpoint))
                }
            }
            daemons = entries.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
            isSearching = !browsers.isEmpty
        }
    }
}

@MainActor
final class GSPClient: ObservableObject {
    private static let heartbeatInterval: TimeInterval = 5
    private static let heartbeatTimeout: TimeInterval = 12

    @Published var connected = false
    @Published var authenticated = false
    @Published var protocolVersion: UInt16?
    @Published var transportCapabilities: UInt32 = 0
    @Published var serverBuildId: String?
    @Published var serverInstanceId: String?
    @Published var lastHeartbeatAck: Date?
    @Published var lastHeartbeatRttMs: Double?
    @Published var sessions: [SessionInfo] = []
    @Published var screen = ScreenState()
    @Published var attachedSessionId: UInt32?
    @Published var attachmentId: UInt64?
    @Published var lastError: String?

    private var connection: NWConnection?
    private var authKey: SymmetricKey?
    private let queue = DispatchQueue(label: "boo-gsp-client", qos: .userInteractive)
    private var heartbeatTimer: DispatchSourceTimer?
    private var lastHeartbeatSent: Date?
    private var pendingHeartbeatToken: UInt64?
    private var desiredAttachedSessionId: UInt32?
    private var desiredAttachmentId: UInt64?
    private var expectedServerInstanceId: String?
    private var connectionGeneration: UInt64 = 0

    private nonisolated static let magic: [UInt8] = [0x47, 0x53]
    private nonisolated static let headerLen = 7

    var handshakeSummary: String? {
        guard let protocolVersion,
              let serverBuildId, !serverBuildId.isEmpty,
              let serverInstanceId, !serverInstanceId.isEmpty else {
            return nil
        }
        let heartbeat = lastHeartbeatRttMs.map { String(format: "hb %.0fms", $0) }
        let attachment = attachmentId.map { "attach 0x" + String($0, radix: 16) }
        let base = [ "proto \(protocolVersion)",
                     "caps 0x\(String(transportCapabilities, radix: 16))",
                     serverBuildId,
                     "srv \(serverInstanceId)",
                     attachment].compactMap { $0 }.joined(separator: " · ")
        if let heartbeat {
            return "\(base) · \(heartbeat)"
        }
        return base
    }

    func connect(host: String, port: UInt16, authKey: String = "") {
        self.authKey = authKey.isEmpty ? nil : SymmetricKey(data: Data(authKey.utf8))
        connectionGeneration &+= 1
        let generation = connectionGeneration
        let params = NWParameters.tcp
        params.allowLocalEndpointReuse = true
        connection = NWConnection(host: NWEndpoint.Host(host), port: NWEndpoint.Port(rawValue: port)!, using: params)
        connection?.stateUpdateHandler = { [weak self] state in
            Task { @MainActor in
                guard let self, self.connectionGeneration == generation else { return }
                switch state {
                case .ready:
                    self.connected = true
                    self.lastError = nil
                    self.readHeader(generation: generation)
                    self.sendAuth()
                case .failed(let error):
                    self.protocolError("Connection failed: \(error)")
                case .cancelled:
                    self.stopHeartbeatLoop()
                    self.connected = false
                default:
                    break
                }
            }
        }
        connection?.start(queue: queue)
    }

    func disconnect() {
        connection?.cancel()
        connection = nil
        connectionGeneration &+= 1
        connected = false
        authenticated = false
        protocolVersion = nil
        transportCapabilities = 0
        serverBuildId = nil
        serverInstanceId = nil
        lastHeartbeatAck = nil
        lastHeartbeatRttMs = nil
        lastHeartbeatSent = nil
        pendingHeartbeatToken = nil
        attachedSessionId = nil
        attachmentId = nil
        desiredAttachedSessionId = nil
        desiredAttachmentId = nil
        sessions = []
        screen = ScreenState()
        stopHeartbeatLoop()
    }

    func listSessions() { sendMessage(type: .listSessions, payload: Data()) }

    func createSession(cols: UInt16 = 120, rows: UInt16 = 36) {
        var payload = Data(count: 4)
        payload.withUnsafeMutableBytes { buf in
            buf.storeBytes(of: cols.littleEndian, as: UInt16.self)
            buf.storeBytes(of: rows.littleEndian, toByteOffset: 2, as: UInt16.self)
        }
        sendMessage(type: .create, payload: payload)
    }

    func attach(sessionId: UInt32) {
        let newAttachmentId = UInt64(Date().timeIntervalSince1970 * 1000)
        desiredAttachedSessionId = sessionId
        desiredAttachmentId = newAttachmentId
        attachedSessionId = sessionId
        attachmentId = newAttachmentId
        sendAttach(sessionId: sessionId, attachmentId: newAttachmentId)
    }

    func configureResumeAttachment(sessionId: UInt32, attachmentId: UInt64) {
        desiredAttachedSessionId = sessionId
        desiredAttachmentId = attachmentId
    }

    func configureTrustedServerInstance(_ instanceId: String?) {
        expectedServerInstanceId = instanceId
    }

    private func sendAttach(sessionId: UInt32, attachmentId: UInt64) {
        var payload = Data(count: 12)
        payload.withUnsafeMutableBytes { $0.storeBytes(of: sessionId.littleEndian, as: UInt32.self) }
        payload.withUnsafeMutableBytes { $0.storeBytes(of: attachmentId.littleEndian, toByteOffset: 4, as: UInt64.self) }
        sendMessage(type: .attach, payload: payload)
    }

    func detach() {
        sendMessage(type: .detach, payload: Data())
        attachedSessionId = nil
        attachmentId = nil
        desiredAttachedSessionId = nil
        desiredAttachmentId = nil
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
        sendMessage(type: .auth, payload: Data())
    }

    private func handleAuthChallenge(_ payload: Data) {
        guard payload.count == 32, let key = authKey else {
            lastError = "Authentication challenge failed"
            return
        }
        let hmac = HMAC<SHA256>.authenticationCode(for: payload, using: key)
        sendMessage(type: .auth, payload: Data(hmac))
    }

    private func sendMessage(type: GSPMessageType, payload: Data) {
        let generation = connectionGeneration
        var header = Data(count: Self.headerLen)
        header[0] = Self.magic[0]
        header[1] = Self.magic[1]
        header[2] = type.rawValue
        let len = UInt32(payload.count).littleEndian
        header.withUnsafeMutableBytes { $0.storeBytes(of: len, toByteOffset: 3, as: UInt32.self) }
        connection?.send(content: header + payload, completion: .contentProcessed { [weak self] error in
            guard let error else { return }
            Task { @MainActor in
                guard let self, self.connectionGeneration == generation else { return }
                self.lastError = "Send failed: \(error)"
            }
        })
    }

    private func readHeader(generation: UInt64) {
        connection?.receive(minimumIncompleteLength: Self.headerLen, maximumLength: Self.headerLen) { [weak self] content, _, isComplete, _ in
            guard let self, self.connectionGeneration == generation else { return }
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

    private func readPayload(type: UInt8, length: Int, generation: UInt64) {
        connection?.receive(minimumIncompleteLength: length, maximumLength: length) { [weak self] content, _, isComplete, _ in
            guard let self, self.connectionGeneration == generation else { return }
            guard let data = content else {
                if isComplete { self.protocolError("Connection closed") }
                return
            }
            self.handleMessage(type: type, payload: data)
            self.readHeader(generation: generation)
        }
    }

    private func handleMessage(type: UInt8, payload: Data) {
        guard let message = GSPMessageType(rawValue: type) else { return }
        switch message {
        case .authChallenge:
            handleAuthChallenge(payload)
        case .authOk:
            if let error = validateAuthOkPayload(payload) {
                protocolError(error)
                return
            }
            applyReducedMessage(.authOk, payload: payload)
        case .authFail:
            applyReducedMessage(.authFail, payload: payload)
        case .sessionList:
            applyReducedMessage(.sessionList, payload: payload)
        case .attached:
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
        validateAuthOkMetadata(payload, authRequired: authKey != nil)
    }

    private var shouldPreserveRemoteStateOnReconnect: Bool {
        desiredAttachedSessionId != nil || !sessions.isEmpty
    }

    private func protocolError(_ message: String) {
        connection?.cancel()
        connection = nil
        connectionGeneration &+= 1
        connected = false
        authenticated = false
        lastHeartbeatAck = nil
        lastHeartbeatRttMs = nil
        lastHeartbeatSent = nil
        pendingHeartbeatToken = nil
        if !shouldPreserveRemoteStateOnReconnect {
            protocolVersion = nil
            transportCapabilities = 0
            serverBuildId = nil
            serverInstanceId = nil
            attachedSessionId = nil
            attachmentId = nil
            sessions = []
            screen = ScreenState()
        }
        lastError = message
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
        lastError = state.lastError
        attachedSessionId = state.attachedSessionId
        attachmentId = state.attachmentId
        applyDecodedSessions(state.sessions)
        if let decodedScreen = state.screen {
            applyDecodedScreen(decodedScreen)
            screen.objectWillChange.send()
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
           attachedSessionId == nil,
           sessions.contains(where: { $0.id == desiredSessionId }) {
            if let expectedServerInstanceId,
               let serverInstanceId,
               expectedServerInstanceId != serverInstanceId {
                lastError = "Server identity changed; refusing automatic resume"
                return
            }
            sendAttach(sessionId: desiredSessionId, attachmentId: desiredAttachmentId)
        }
    }

    private func handleClipboard(_ data: Data) {
        guard let encoded = String(data: data, encoding: .utf8),
              let bytes = Data(base64Encoded: encoded),
              let string = String(data: bytes, encoding: .utf8) else { return }
        UIPasteboard.general.string = string
    }
}
