import Foundation
import Network
import CryptoKit

private struct ValidationAuthOkMetadata {
    let protocolVersion: UInt16
    let transportCapabilities: UInt32
    let serverBuildId: String?
    let serverInstanceId: String?
    let serverIdentityId: String?
}

private let validationCapabilityHmacAuth: UInt32 = 1 << 0
private let validationCapabilityHeartbeat: UInt32 = 1 << 4
private let validationCapabilityAttachmentResume: UInt32 = 1 << 5
private let validationCapabilityDaemonIdentity: UInt32 = 1 << 6

private func decodeValidationAuthOkMetadata(_ payload: Data) -> ValidationAuthOkMetadata? {
    guard payload.count >= 6 else { return nil }
    let protocolVersion = payload.withUnsafeBytes {
        UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt16.self))
    }
    let transportCapabilities = payload.withUnsafeBytes {
        UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 2, as: UInt32.self))
    }
    guard payload.count >= 8 else {
        return ValidationAuthOkMetadata(
            protocolVersion: protocolVersion,
            transportCapabilities: transportCapabilities,
            serverBuildId: nil,
            serverInstanceId: nil,
            serverIdentityId: nil
        )
    }
    let buildLength = payload.withUnsafeBytes {
        Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 6, as: UInt16.self)))
    }
    guard payload.count >= 8 + buildLength else { return nil }
    let instanceLengthOffset = 8 + buildLength
    let serverBuildId = String(data: payload[8..<(8 + buildLength)], encoding: .utf8)
    guard payload.count >= instanceLengthOffset + 2 else {
        return ValidationAuthOkMetadata(
            protocolVersion: protocolVersion,
            transportCapabilities: transportCapabilities,
            serverBuildId: serverBuildId,
            serverInstanceId: nil,
            serverIdentityId: nil
        )
    }
    let instanceLength = payload.withUnsafeBytes {
        Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: instanceLengthOffset, as: UInt16.self)))
    }
    guard payload.count >= instanceLengthOffset + 2 + instanceLength else { return nil }
    let identityLengthOffset = instanceLengthOffset + 2 + instanceLength
    let serverInstanceId = String(
        data: payload[(instanceLengthOffset + 2)..<(instanceLengthOffset + 2 + instanceLength)],
        encoding: .utf8
    )
    guard payload.count >= identityLengthOffset + 2 else {
        return ValidationAuthOkMetadata(
            protocolVersion: protocolVersion,
            transportCapabilities: transportCapabilities,
            serverBuildId: serverBuildId,
            serverInstanceId: serverInstanceId,
            serverIdentityId: nil
        )
    }
    let identityLength = payload.withUnsafeBytes {
        Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: identityLengthOffset, as: UInt16.self)))
    }
    guard payload.count >= identityLengthOffset + 2 + identityLength else { return nil }
    return ValidationAuthOkMetadata(
        protocolVersion: protocolVersion,
        transportCapabilities: transportCapabilities,
        serverBuildId: serverBuildId,
        serverInstanceId: serverInstanceId,
        serverIdentityId: String(
            data: payload[(identityLengthOffset + 2)..<(identityLengthOffset + 2 + identityLength)],
            encoding: .utf8
        )
    )
}

private func validateValidationAuthOkMetadata(_ payload: Data, authRequired: Bool) -> String? {
    guard let metadata = decodeValidationAuthOkMetadata(payload) else {
        return "Remote handshake is malformed"
    }
    if metadata.protocolVersion != 1 {
        return "Unsupported remote protocol version: \(metadata.protocolVersion)"
    }
    if authRequired && (metadata.transportCapabilities & validationCapabilityHmacAuth) == 0 {
        return "Remote server does not advertise HMAC authentication"
    }
    if (metadata.transportCapabilities & validationCapabilityHeartbeat) == 0 {
        return "Remote server does not advertise heartbeat support"
    }
    if (metadata.transportCapabilities & validationCapabilityAttachmentResume) == 0 {
        return "Remote server does not advertise attachment resume support"
    }
    if (metadata.transportCapabilities & validationCapabilityDaemonIdentity) == 0 {
        return "Remote server does not advertise daemon identity support"
    }
    if metadata.serverBuildId?.isEmpty != false {
        return "Remote handshake is missing server build metadata"
    }
    if metadata.serverInstanceId?.isEmpty != false {
        return "Remote handshake is missing server instance metadata"
    }
    if metadata.serverIdentityId?.isEmpty != false {
        return "Remote handshake is missing server identity metadata"
    }
    return nil
}

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
    case heartbeatAck = 0x92
}

final class RemoteValidator {
    private let magic: [UInt8] = [0x47, 0x53]
    private let queue = DispatchQueue(label: "boo-ios-remote-validator")
    private let lock = NSLock()

    private var connection: NWConnection?
    private var authKey: SymmetricKey?

    private var connected = false
    private var authenticated = false
    private var protocolVersion: UInt16?
    private var transportCapabilities: UInt32 = 0
    private var serverBuildId: String?
    private var serverInstanceId: String?
    private var serverIdentityId: String?
    private var heartbeatAckReceived = false
    private var expectedHeartbeatPayload = Data()
    private var sessionListReceived = false
    private var sessions: [DecodedWireSessionInfo] = []
    private var attachedSessionId: UInt32?
    private var attachmentId: UInt64?
    private var resumeToken: UInt64?
    private var createdSessionId: UInt32?
    private var screenState: DecodedWireScreenState?
    private var lastScreenText = ""
    private var screenUpdateReceived = false
    private var lastError: String?
    private var discoveredEndpoint: NWEndpoint?
    private var messageTrace: [String] = []
    private var connectedHost: String?
    private var connectedPort: UInt16?
    private var connectionGeneration: UInt64 = 0

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
        try startConnection(host: host, port: port)
        if authKey != nil {
            sendMessage(type: .auth, payload: Data())
            try waitUntil("authentication") { self.authenticated }
        } else {
            authenticated = true
        }
        heartbeatAckReceived = false
        expectedHeartbeatPayload = Data(withUnsafeBytes(of: UInt64(0x424f4f5f50494e47).littleEndian, Array.init))
        sendMessage(type: .heartbeat, payload: expectedHeartbeatPayload)
        try waitUntil("heartbeat acknowledgement") { self.heartbeatAckReceived }
    }

    func validateAuthenticationFailure(host: String, port: UInt16) throws {
        guard authKey != nil else {
            throw ValidationError("authentication-failure validation requires an auth key")
        }
        try startConnection(host: host, port: port)
        sendMessage(type: .auth, payload: Data())
        try waitUntilAnyError(["authentication failed", "connection closed"])
    }

    private func startConnection(host: String, port: UInt16) throws {
        connectedHost = host
        connectedPort = port
        connectionGeneration &+= 1
        let generation = connectionGeneration
        let semaphore = DispatchSemaphore(value: 0)
        let params = NWParameters.tcp
        params.allowLocalEndpointReuse = true
        let conn = NWConnection(
            host: NWEndpoint.Host(host),
            port: NWEndpoint.Port(rawValue: port)!,
            using: params
        )
        conn.stateUpdateHandler = { [weak self] state in
            guard let self, self.connectionGeneration == generation else { return }
            switch state {
            case .ready:
                self.lock.lock()
                self.connected = true
                self.lock.unlock()
                self.readHeader(generation: generation)
                semaphore.signal()
            case .failed(let error):
                self.setError("connection failed: \(error)")
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
    }

    func validateRoundTrip() throws {
        sessionListReceived = false
        sendMessage(type: .listSessions, payload: Data())
        try waitUntil("session list") { self.sessionListReceived }

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

        let expectedAttachmentId = UInt64(0xB001D00DCAFEBEEF)
        var attachPayload = Data(count: 12)
        attachPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: sessionId.littleEndian, as: UInt32.self)
            bytes.storeBytes(of: expectedAttachmentId.littleEndian, toByteOffset: 4, as: UInt64.self)
        }
        sendMessage(type: .attach, payload: attachPayload)
        try waitUntil("attach acknowledgement") {
            self.attachedSessionId == sessionId
                && self.attachmentId == expectedAttachmentId
                && self.resumeToken != nil
        }
        guard let resumeToken = resumeToken else {
            throw ValidationError("server did not return a resume token")
        }
        try waitUntil("initial terminal screen update after attach", timeout: 8) {
            self.screenUpdateReceived
        }

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

        try reconnectForResumeValidation()
        sessionListReceived = false
        sendMessage(type: .listSessions, payload: Data())
        try waitUntil("session list after reconnect") { self.sessionListReceived }
        var wrongResumeAttachPayload = Data(count: 20)
        wrongResumeAttachPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: sessionId.littleEndian, as: UInt32.self)
            bytes.storeBytes(of: expectedAttachmentId.littleEndian, toByteOffset: 4, as: UInt64.self)
            bytes.storeBytes(of: (resumeToken ^ 0xffff).littleEndian, toByteOffset: 12, as: UInt64.self)
        }
        sendMessage(type: .attach, payload: wrongResumeAttachPayload)
        try waitUntilError("attachment resume token mismatch")
        clearLastError()

        var resumeAttachPayload = Data(count: 20)
        resumeAttachPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: sessionId.littleEndian, as: UInt32.self)
            bytes.storeBytes(of: expectedAttachmentId.littleEndian, toByteOffset: 4, as: UInt64.self)
            bytes.storeBytes(of: resumeToken.littleEndian, toByteOffset: 12, as: UInt64.self)
        }
        sendMessage(type: .attach, payload: resumeAttachPayload)
        try waitUntil("attach acknowledgement after reconnect") {
            self.attachedSessionId == sessionId
                && self.attachmentId == expectedAttachmentId
                && self.resumeToken == resumeToken
        }
        try waitUntil("terminal state restore after reconnect", timeout: 8) {
            self.lastScreenText.contains(marker)
        }

        sendMessage(type: .detach, payload: Data())
        sendMessage(type: .destroy, payload: attachPayload)
    }

    func disconnect() {
        connection?.cancel()
        connection = nil
        connectionGeneration &+= 1
    }

    private func reconnectForResumeValidation() throws {
        disconnect()
        lock.lock()
        connected = false
        authenticated = false
        protocolVersion = nil
        transportCapabilities = 0
        serverBuildId = nil
        serverInstanceId = nil
        serverIdentityId = nil
        heartbeatAckReceived = false
        sessionListReceived = false
        attachedSessionId = nil
        attachmentId = nil
        resumeToken = nil
        screenUpdateReceived = false
        lastError = nil
        messageTrace.removeAll()
        lock.unlock()
        Thread.sleep(forTimeInterval: 0.2)
        guard let connectedHost, let connectedPort else {
            throw ValidationError("missing reconnect target")
        }
        try connect(host: connectedHost, port: connectedPort)
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

    private func readHeader(generation: UInt64) {
        connection?.receive(minimumIncompleteLength: 7, maximumLength: 7) { [weak self] content, _, complete, error in
            guard let self, self.connectionGeneration == generation else { return }
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
                self.readHeader(generation: generation)
            } else {
                self.readPayload(type: type, length: length, generation: generation)
            }
        }
    }

    private func readPayload(type: UInt8, length: Int, generation: UInt64) {
        connection?.receive(minimumIncompleteLength: length, maximumLength: length) { [weak self] content, _, complete, error in
            guard let self, self.connectionGeneration == generation else { return }
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
            self.readHeader(generation: generation)
        }
    }

    private func handleMessage(type: UInt8, payload: Data) {
        guard let message = WireMessageType(rawValue: type) else {
            return
        }
        lock.lock()
        defer { lock.unlock() }
        messageTrace.append(String(describing: message))
        if messageTrace.count > 16 {
            messageTrace.removeFirst(messageTrace.count - 16)
        }
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
            if let metadata = decodeValidationAuthOkMetadata(payload) {
                protocolVersion = metadata.protocolVersion
                transportCapabilities = metadata.transportCapabilities
                serverBuildId = metadata.serverBuildId
                serverInstanceId = metadata.serverInstanceId
                serverIdentityId = metadata.serverIdentityId
            }
            if let error = validateValidationAuthOkMetadata(payload, authRequired: authKey != nil) {
                lastError = error
                return
            }
            authenticated = true
        case .authFail:
            lastError = "authentication failed"
        case .heartbeatAck:
            if payload != expectedHeartbeatPayload {
                lastError = "heartbeat acknowledgement payload mismatch"
                return
            }
            heartbeatAckReceived = true
        case .sessionList:
            sessionListReceived = true
            sessions = WireCodec.decodeSessionList(payload)
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
            attachmentId = payload.count >= 12 ? payload.withUnsafeBytes {
                UInt64(littleEndian: $0.loadUnaligned(fromByteOffset: 4, as: UInt64.self))
            } : nil
            resumeToken = payload.count >= 20 ? payload.withUnsafeBytes {
                UInt64(littleEndian: $0.loadUnaligned(fromByteOffset: 12, as: UInt64.self))
            } : nil
        case .fullState:
            screenUpdateReceived = true
            if let screen = WireCodec.decodeFullState(payload) {
                screenState = screen
                lastScreenText = WireCodec.screenText(from: screen)
            }
        case .delta:
            screenUpdateReceived = true
            if var screen = screenState, WireCodec.applyDelta(payload, to: &screen) {
                screenState = screen
                lastScreenText = WireCodec.screenText(from: screen)
            }
        case .detached, .sessionExited:
            attachedSessionId = nil
            attachmentId = nil
        case .errorMsg:
            lastError = String(data: payload, encoding: .utf8) ?? "remote error"
        default:
            break
        }
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
        lock.lock()
        let screenSnippet = lastScreenText
            .replacingOccurrences(of: "\n", with: "\\n")
            .prefix(160)
        let stateSummary = "authenticated=\(authenticated) heartbeatAckReceived=\(heartbeatAckReceived) sessionListReceived=\(sessionListReceived) sessions=\(sessions.count) createdSessionId=\(String(describing: createdSessionId)) attachedSessionId=\(String(describing: attachedSessionId)) attachmentId=\(String(describing: attachmentId)) resumeToken=\(String(describing: resumeToken)) buildId=\(serverBuildId ?? "nil") serverIdentityId=\(serverIdentityId ?? "nil") serverInstanceId=\(serverInstanceId ?? "nil") screenUpdateReceived=\(screenUpdateReceived) screen=\"\(screenSnippet)\" trace=\(messageTrace.joined(separator: ","))"
        lock.unlock()
        throw ValidationError("timed out waiting for \(description) (\(stateSummary))")
    }

    private func throwIfError() throws {
        lock.lock()
        let error = lastError
        lock.unlock()
        if let error {
            throw ValidationError(error)
        }
    }

    private func waitUntilError(_ expected: String, timeout: TimeInterval = 5) throws {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            lock.lock()
            let error = lastError
            lock.unlock()
            if error == expected {
                return
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
        throw ValidationError("timed out waiting for expected remote error: \(expected)")
    }

    private func waitUntilAnyError(_ expectedErrors: [String], timeout: TimeInterval = 5) throws {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            lock.lock()
            let error = lastError
            lock.unlock()
            if let error, expectedErrors.contains(error) {
                return
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
        throw ValidationError(
            "timed out waiting for one of expected remote errors: \(expectedErrors.joined(separator: ", "))"
        )
    }

    private func clearLastError() {
        lock.lock()
        lastError = nil
        lock.unlock()
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

func resolveRemoteValidatorArgs() -> (host: String, port: UInt16, authKey: String, checkDiscovery: Bool, expectAuthFailure: Bool) {
    var host = "127.0.0.1"
    var port: UInt16 = 7337
    var authKey = ""
    var checkDiscovery = false
    var expectAuthFailure = false
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
        case "--expect-auth-failure":
            expectAuthFailure = true
        default:
            break
        }
        index += 1
    }
    return (host, port, authKey, checkDiscovery, expectAuthFailure)
}
