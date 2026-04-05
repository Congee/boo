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
    @Published var connected = false
    @Published var authenticated = false
    @Published var sessions: [SessionInfo] = []
    @Published var screen = ScreenState()
    @Published var attachedSessionId: UInt32?
    @Published var lastError: String?

    private var connection: NWConnection?
    private var authKey: SymmetricKey?
    private let queue = DispatchQueue(label: "boo-gsp-client", qos: .userInteractive)

    private nonisolated static let magic: [UInt8] = [0x47, 0x53]
    private nonisolated static let headerLen = 7

    func connect(host: String, port: UInt16, authKey: String = "") {
        self.authKey = authKey.isEmpty ? nil : SymmetricKey(data: Data(authKey.utf8))
        let hasAuth = !authKey.isEmpty
        let params = NWParameters.tcp
        params.allowLocalEndpointReuse = true
        connection = NWConnection(host: NWEndpoint.Host(host), port: NWEndpoint.Port(rawValue: port)!, using: params)
        connection?.stateUpdateHandler = { [weak self] state in
            Task { @MainActor in
                switch state {
                case .ready:
                    self?.connected = true
                    self?.lastError = nil
                    self?.readHeader()
                    if hasAuth {
                        self?.sendAuth()
                    } else {
                        self?.authenticated = true
                    }
                case .failed(let error):
                    self?.connected = false
                    self?.lastError = "Connection failed: \(error)"
                case .cancelled:
                    self?.connected = false
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
        connected = false
        authenticated = false
        attachedSessionId = nil
        sessions = []
        screen = ScreenState()
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
        attachedSessionId = sessionId
        var payload = Data(count: 4)
        payload.withUnsafeMutableBytes { $0.storeBytes(of: sessionId.littleEndian, as: UInt32.self) }
        sendMessage(type: .attach, payload: payload)
    }

    func detach() {
        sendMessage(type: .detach, payload: Data())
        attachedSessionId = nil
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
        var header = Data(count: Self.headerLen)
        header[0] = Self.magic[0]
        header[1] = Self.magic[1]
        header[2] = type.rawValue
        let len = UInt32(payload.count).littleEndian
        header.withUnsafeMutableBytes { $0.storeBytes(of: len, toByteOffset: 3, as: UInt32.self) }
        connection?.send(content: header + payload, completion: .contentProcessed { [weak self] error in
            guard let error else { return }
            Task { @MainActor in self?.lastError = "Send failed: \(error)" }
        })
    }

    private func readHeader() {
        connection?.receive(minimumIncompleteLength: Self.headerLen, maximumLength: Self.headerLen) { [weak self] content, _, isComplete, _ in
            guard let self, let data = content, data.count == Self.headerLen else {
                if isComplete { Task { @MainActor in self?.disconnect() } }
                return
            }
            guard data[0] == Self.magic[0], data[1] == Self.magic[1] else {
                Task { @MainActor in
                    self.lastError = "Invalid protocol header"
                    self.disconnect()
                }
                return
            }
            let type = data[2]
            let payloadLen = data.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 3, as: UInt32.self)) }
            if payloadLen == 0 {
                Task { @MainActor in
                    self.handleMessage(type: type, payload: Data())
                    self.readHeader()
                }
            } else {
                Task { @MainActor in
                    self.readPayload(type: type, length: Int(payloadLen))
                }
            }
        }
    }

    private func readPayload(type: UInt8, length: Int) {
        connection?.receive(minimumIncompleteLength: length, maximumLength: length) { [weak self] content, _, isComplete, _ in
            guard let self, let data = content else {
                if isComplete { Task { @MainActor in self?.disconnect() } }
                return
            }
            Task { @MainActor in
                self.handleMessage(type: type, payload: data)
                self.readHeader()
            }
        }
    }

    private func handleMessage(type: UInt8, payload: Data) {
        guard let message = GSPMessageType(rawValue: type) else { return }
        switch message {
        case .authChallenge:
            handleAuthChallenge(payload)
        case .authOk:
            authenticated = true
            lastError = nil
            listSessions()
        case .authFail:
            lastError = "Authentication failed"
        case .sessionList:
            parseSessionList(payload)
        case .attached:
            guard payload.count >= 4 else { break }
            attachedSessionId = payload.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
            }
        case .sessionCreated:
            guard payload.count >= 4 else { break }
            let sessionId = payload.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self)) }
            attach(sessionId: sessionId)
        case .fullState:
            applyFullState(payload)
        case .delta:
            applyDelta(payload)
        case .detached, .sessionExited:
            attachedSessionId = nil
        case .errorMsg:
            lastError = String(data: payload, encoding: .utf8) ?? "Remote error"
        case .clipboard:
            handleClipboard(payload)
        default:
            break
        }
    }

    private func parseSessionList(_ data: Data) {
        sessions = WireCodec.decodeSessionList(data).map {
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

    private func applyFullState(_ data: Data) {
        guard let decoded = WireCodec.decodeFullState(data) else { return }
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
    }

    private func applyDelta(_ data: Data) {
        var decoded = DecodedWireScreenState(
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
            cursorVisible: screen.cursorVisible
        )
        guard WireCodec.applyDelta(data, to: &decoded) else { return }
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
        screen.objectWillChange.send()
    }

    private func handleClipboard(_ data: Data) {
        guard let encoded = String(data: data, encoding: .utf8),
              let bytes = Data(base64Encoded: encoded),
              let string = String(data: bytes, encoding: .utf8) else { return }
        UIPasteboard.general.string = string
    }
}
