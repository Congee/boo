import Foundation
import Network
import CryptoKit

enum WireMessageType: UInt8 {
    case auth = 0x01
    case listSessions = 0x02
    case attach = 0x03
    case detach = 0x04
    case create = 0x05
    case input = 0x06
    case resize = 0x07
    case destroy = 0x08
    case authChallenge = 0x09

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
}

struct SessionInfo {
    let id: UInt32
    let name: String
    let title: String
    let pwd: String
    let attached: Bool
    let childExited: Bool
}

final class RemoteValidator {
    private let magic: [UInt8] = [0x47, 0x53]
    private let queue = DispatchQueue(label: "boo-ios-remote-validator")
    private let lock = NSLock()

    private var connection: NWConnection?
    private var authKey: SymmetricKey?

    private var connected = false
    private var authenticated = false
    private var sessions: [SessionInfo] = []
    private var attachedSessionId: UInt32?
    private var createdSessionId: UInt32?
    private var lastScreenText = ""
    private var lastError: String?
    private var discoveredEndpoint: NWEndpoint?

    init(authKey: String) {
        self.authKey = authKey.isEmpty ? nil : SymmetricKey(data: Data(authKey.utf8))
    }

    func browse(serviceType: String = "_boo._tcp", timeout: TimeInterval = 3.0) -> NWEndpoint? {
        let semaphore = DispatchSemaphore(value: 0)
        let browser = NWBrowser(
            for: .bonjour(type: serviceType, domain: nil),
            using: NWParameters()
        )
        browser.stateUpdateHandler = { state in
            if case .failed = state {
                semaphore.signal()
            }
        }
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            if let first = results.first {
                self?.lock.lock()
                self?.discoveredEndpoint = first.endpoint
                self?.lock.unlock()
                semaphore.signal()
            }
        }
        browser.start(queue: queue)
        let deadline = DispatchTime.now() + timeout
        _ = semaphore.wait(timeout: deadline)
        browser.cancel()
        lock.lock()
        defer { lock.unlock() }
        return discoveredEndpoint
    }

    func connect(host: String, port: UInt16) throws {
        let semaphore = DispatchSemaphore(value: 0)
        let params = NWParameters.tcp
        params.allowLocalEndpointReuse = true
        let conn = NWConnection(
            host: NWEndpoint.Host(host),
            port: NWEndpoint.Port(rawValue: port)!,
            using: params
        )
        conn.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready:
                self?.lock.lock()
                self?.connected = true
                self?.lock.unlock()
                self?.readHeader()
                semaphore.signal()
            case .failed(let error):
                self?.setError("connection failed: \(error)")
                semaphore.signal()
            default:
                break
            }
        }
        self.connection = conn
        conn.start(queue: queue)
        if semaphore.wait(timeout: .now() + 5) == .timedOut {
            throw ValidationError("timed out connecting to Boo daemon")
        }
        try throwIfError()
        if authKey != nil {
            sendMessage(type: .auth, payload: Data())
            try waitUntil("authentication") { self.authenticated }
        } else {
            authenticated = true
        }
    }

    func validateRoundTrip() throws {
        sendMessage(type: .listSessions, payload: Data())
        try waitUntil("session list") { !self.sessions.isEmpty || self.lastError != nil || self.connected }

        var createPayload = Data(count: 4)
        createPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(120).littleEndian, as: UInt16.self)
            bytes.storeBytes(of: UInt16(36).littleEndian, toByteOffset: 2, as: UInt16.self)
        }
        sendMessage(type: .create, payload: createPayload)
        try waitUntil("session creation") { self.createdSessionId != nil }
        guard let sessionId = createdSessionId else {
            throw ValidationError("server did not return a created session id")
        }

        var attachPayload = Data(count: 4)
        attachPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: sessionId.littleEndian, as: UInt32.self)
        }
        sendMessage(type: .attach, payload: attachPayload)
        try waitUntil("attach acknowledgement") { self.attachedSessionId == sessionId }

        var resizePayload = Data(count: 4)
        resizePayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(100).littleEndian, as: UInt16.self)
            bytes.storeBytes(of: UInt16(30).littleEndian, toByteOffset: 2, as: UInt16.self)
        }
        sendMessage(type: .resize, payload: resizePayload)

        let marker = "BOO_IOS_REMOTE_VALIDATION"
        sendMessage(type: .input, payload: Data("printf '\(marker)\\n'\r".utf8))
        try waitUntil("terminal state update containing validation marker", timeout: 8) {
            self.lastScreenText.contains(marker)
        }

        sendMessage(type: .detach, payload: Data())
        sendMessage(type: .destroy, payload: attachPayload)
    }

    func disconnect() {
        connection?.cancel()
        connection = nil
    }

    private func sendMessage(type: WireMessageType, payload: Data) {
        var header = Data(count: 7)
        header[0] = magic[0]
        header[1] = magic[1]
        header[2] = type.rawValue
        header.withUnsafeMutableBytes {
            $0.storeBytes(of: UInt32(payload.count).littleEndian, toByteOffset: 3, as: UInt32.self)
        }
        connection?.send(content: header + payload, completion: .contentProcessed { [weak self] error in
            if let error {
                self?.setError("send failed: \(error)")
            }
        })
    }

    private func readHeader() {
        connection?.receive(minimumIncompleteLength: 7, maximumLength: 7) { [weak self] content, _, complete, error in
            guard let self else { return }
            if let error {
                self.setError("receive failed: \(error)")
                return
            }
            guard let content, content.count == 7 else {
                if complete {
                    self.setError("connection closed")
                }
                return
            }
            guard content[0] == self.magic[0], content[1] == self.magic[1] else {
                self.setError("invalid protocol header")
                return
            }
            let type = content[2]
            let length = content.withUnsafeBytes {
                Int(UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 3, as: UInt32.self)))
            }
            if length == 0 {
                self.handleMessage(type: type, payload: Data())
                self.readHeader()
            } else {
                self.readPayload(type: type, length: length)
            }
        }
    }

    private func readPayload(type: UInt8, length: Int) {
        connection?.receive(minimumIncompleteLength: length, maximumLength: length) { [weak self] content, _, complete, error in
            guard let self else { return }
            if let error {
                self.setError("receive failed: \(error)")
                return
            }
            guard let content else {
                if complete {
                    self.setError("connection closed")
                }
                return
            }
            self.handleMessage(type: type, payload: content)
            self.readHeader()
        }
    }

    private func handleMessage(type: UInt8, payload: Data) {
        guard let message = WireMessageType(rawValue: type) else {
            return
        }
        lock.lock()
        defer { lock.unlock() }
        switch message {
        case .authChallenge:
            guard payload.count == 32, let key = authKey else {
                lastError = "authentication challenge invalid"
                return
            }
            let mac = HMAC<SHA256>.authenticationCode(for: payload, using: key)
            lock.unlock()
            sendMessage(type: .auth, payload: Data(mac))
            lock.lock()
        case .authOk:
            authenticated = true
        case .authFail:
            lastError = "authentication failed"
        case .sessionList:
            sessions = parseSessionList(payload)
        case .sessionCreated:
            guard payload.count >= 4 else { return }
            createdSessionId = payload.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
            }
        case .attached:
            guard payload.count >= 4 else { return }
            attachedSessionId = payload.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
            }
        case .fullState:
            lastScreenText = decodeScreenText(payload)
        case .detached, .sessionExited:
            attachedSessionId = nil
        case .errorMsg:
            lastError = String(data: payload, encoding: .utf8) ?? "remote error"
        default:
            break
        }
    }

    private func parseSessionList(_ data: Data) -> [SessionInfo] {
        guard data.count >= 4 else { return [] }
        let count = data.withUnsafeBytes {
            Int(UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self)))
        }
        var offset = 4
        var items: [SessionInfo] = []
        func readString() -> String {
            guard offset + 2 <= data.count else { return "" }
            let len = data.withUnsafeBytes {
                Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: UInt16.self)))
            }
            offset += 2
            guard offset + len <= data.count else { return "" }
            let value = String(data: data[offset..<(offset + len)], encoding: .utf8) ?? ""
            offset += len
            return value
        }
        for _ in 0..<count {
            guard offset + 4 <= data.count else { break }
            let id = data.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
            }
            offset += 4
            let name = readString()
            let title = readString()
            let pwd = readString()
            guard offset < data.count else { break }
            let flags = data[offset]
            offset += 1
            items.append(SessionInfo(
                id: id,
                name: name,
                title: title,
                pwd: pwd,
                attached: (flags & 0x01) != 0,
                childExited: (flags & 0x02) != 0
            ))
        }
        return items
    }

    private func decodeScreenText(_ data: Data) -> String {
        guard data.count >= 12 else { return "" }
        let rows = data.withUnsafeBytes {
            Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt16.self)))
        }
        let cols = data.withUnsafeBytes {
            Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 2, as: UInt16.self)))
        }
        var offset = 12
        var text = ""
        for row in 0..<rows {
            for _ in 0..<cols {
                guard offset + 12 <= data.count else { break }
                let codepoint = data.withUnsafeBytes {
                    UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
                }
                offset += 12
                if codepoint == 0 {
                    text.append(" ")
                } else if let scalar = UnicodeScalar(codepoint) {
                    text.append(Character(scalar))
                }
            }
            if row + 1 < rows {
                text.append("\n")
            }
        }
        return text
    }

    private func waitUntil(_ description: String, timeout: TimeInterval = 5, predicate: @escaping () -> Bool) throws {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            try throwIfError()
            lock.lock()
            let done = predicate()
            lock.unlock()
            if done {
                return
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
        throw ValidationError("timed out waiting for \(description)")
    }

    private func throwIfError() throws {
        lock.lock()
        let error = lastError
        lock.unlock()
        if let error {
            throw ValidationError(error)
        }
    }

    private func setError(_ message: String) {
        lock.lock()
        lastError = message
        lock.unlock()
    }
}

struct ValidationError: Error, CustomStringConvertible {
    let description: String
    init(_ description: String) { self.description = description }
}

func resolveArgs() -> (host: String, port: UInt16, authKey: String, checkDiscovery: Bool) {
    var host = "127.0.0.1"
    var port: UInt16 = 7337
    var authKey = ""
    var checkDiscovery = false
    var index = 1
    while index < CommandLine.arguments.count {
        switch CommandLine.arguments[index] {
        case "--host":
            index += 1
            host = CommandLine.arguments[index]
        case "--port":
            index += 1
            port = UInt16(CommandLine.arguments[index]) ?? 7337
        case "--auth-key":
            index += 1
            authKey = CommandLine.arguments[index]
        case "--check-discovery":
            checkDiscovery = true
        default:
            break
        }
        index += 1
    }
    return (host, port, authKey, checkDiscovery)
}

let args = resolveArgs()
let validator = RemoteValidator(authKey: args.authKey)

do {
    if args.checkDiscovery {
        if let endpoint = validator.browse() {
            FileHandle.standardError.write(Data("discovered Bonjour endpoint: \(endpoint)\n".utf8))
        } else {
            FileHandle.standardError.write(Data("warning: Bonjour discovery did not resolve within timeout\n".utf8))
        }
    }
    try validator.connect(host: args.host, port: args.port)
    try validator.validateRoundTrip()
    validator.disconnect()
    print("iOS remote daemon validation passed")
} catch {
    validator.disconnect()
    FileHandle.standardError.write(Data("validation failed: \(error)\n".utf8))
    exit(1)
}
