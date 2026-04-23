import Foundation
import Network

private struct ValidationAuthOkMetadata {
    let protocolVersion: UInt16
    let transportCapabilities: UInt32
    let serverBuildId: String?
    let serverInstanceId: String?
    let serverIdentityId: String?
}

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

private func validateValidationAuthOkMetadata(_ payload: Data) -> String? {
    guard let metadata = decodeValidationAuthOkMetadata(payload) else {
        return "Remote handshake is malformed"
    }
    if metadata.protocolVersion != 1 {
        return "Unsupported remote protocol version: \(metadata.protocolVersion)"
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
    case listTabs = 0x02
    case attach = 0x03
    case detach = 0x04
    case create = 0x05
    case input = 0x06
    case resize = 0x07
    case destroy = 0x08
    case heartbeat = 0x11

    case authOk = 0x80
    case authFail = 0x81
    case tabList = 0x82
    case fullState = 0x83
    case delta = 0x84
    case attached = 0x85
    case detached = 0x86
    case errorMsg = 0x87
    case tabCreated = 0x88
    case tabExited = 0x89
    case uiRuntimeState = 0x8d
    case uiAppearance = 0x8e
    case heartbeatAck = 0x92
}

final class RemoteValidator {
    private let magic: [UInt8] = [0x47, 0x53]
    private let queue = DispatchQueue(label: "boo-ios-remote-validator")
    private let lock = NSLock()

    private var connection: NWConnection?

    private var connected = false
    private var authenticated = false
    private var protocolVersion: UInt16?
    private var transportCapabilities: UInt32 = 0
    private var serverBuildId: String?
    private var serverInstanceId: String?
    private var serverIdentityId: String?
    private var heartbeatAckReceived = false
    private var expectedHeartbeatPayload = Data()
    private var tabListReceived = false
    private var tabs: [DecodedWireTabInfo] = []
    private var attachedTabId: UInt32?
    private var attachmentId: UInt64?
    private var resumeToken: UInt64?
    private var createdTabId: UInt32?
    private var screenState: DecodedWireScreenState?
    private var lastScreenText = ""
    private var screenUpdateReceived = false
    private var lastError: String?
    private var discoveredEndpoint: NWEndpoint?
    private var messageTrace: [String] = []
    private var connectedHost: String?
    private var connectedPort: UInt16?
    private var connectionGeneration: UInt64 = 0

    init() {}

    func browse(serviceType: String = "_boo._udp", timeout: TimeInterval = 3.0) -> NWEndpoint? {
        let semaphore = DispatchSemaphore(value: 0)
        let browser = NWBrowser(
            for: .bonjour(type: serviceType, domain: nil),
            using: NWParameters.udp
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
        sendMessage(type: .auth, payload: Data())
        try waitUntil("authentication") { self.authenticated }
        heartbeatAckReceived = false
        expectedHeartbeatPayload = Data(withUnsafeBytes(of: UInt64(0x424f4f5f50494e47).littleEndian, Array.init))
        sendMessage(type: .heartbeat, payload: expectedHeartbeatPayload)
        try waitUntil("heartbeat acknowledgement") { self.heartbeatAckReceived }
    }

    private func startConnection(host: String, port: UInt16) throws {
        connectedHost = host
        connectedPort = port
        connectionGeneration &+= 1
        let generation = connectionGeneration
        let semaphore = DispatchSemaphore(value: 0)
        let params = makeQUICParameters()
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

    private func makeQUICParameters() -> NWParameters {
        let options = NWProtocolQUIC.Options(alpn: ["boo-remote"])
        options.direction = .bidirectional
        options.idleTimeout = 5_000
        sec_protocol_options_set_verify_block(options.securityProtocolOptions, { _, _, complete in
            complete(true)
        }, queue)
        let params = NWParameters(quic: options)
        params.allowLocalEndpointReuse = true
        params.includePeerToPeer = true
        return params
    }

    func validateRoundTrip() throws {
        tabListReceived = false
        sendMessage(type: .listTabs, payload: Data())
        try waitUntil("tab list") { self.tabListReceived }

        var createPayload = Data(count: 4)
        createPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: UInt16(120).littleEndian, as: UInt16.self)
            bytes.storeBytes(of: UInt16(36).littleEndian, toByteOffset: 2, as: UInt16.self)
        }
        sendMessage(type: .create, payload: createPayload)
        try waitUntil("tab creation") { self.createdTabId != nil }
        guard let tabId = createdTabId else {
            throw ValidationError("server did not return a created tab id")
        }

        let expectedAttachmentId = UInt64(0xB001D00DCAFEBEEF)
        var attachPayload = Data(count: 12)
        attachPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: tabId.littleEndian, as: UInt32.self)
            bytes.storeBytes(of: expectedAttachmentId.littleEndian, toByteOffset: 4, as: UInt64.self)
        }
        sendMessage(type: .attach, payload: attachPayload)
        try waitUntil("attach acknowledgement") {
            self.attachedTabId == tabId
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
        tabListReceived = false
        sendMessage(type: .listTabs, payload: Data())
        try waitUntil("tab list after reconnect") { self.tabListReceived }
        var wrongResumeAttachPayload = Data(count: 20)
        wrongResumeAttachPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: tabId.littleEndian, as: UInt32.self)
            bytes.storeBytes(of: expectedAttachmentId.littleEndian, toByteOffset: 4, as: UInt64.self)
            bytes.storeBytes(of: (resumeToken ^ 0xffff).littleEndian, toByteOffset: 12, as: UInt64.self)
        }
        sendMessage(type: .attach, payload: wrongResumeAttachPayload)
        try waitUntilError("attachment resume token mismatch")
        clearLastError()

        var resumeAttachPayload = Data(count: 20)
        resumeAttachPayload.withUnsafeMutableBytes { bytes in
            bytes.storeBytes(of: tabId.littleEndian, as: UInt32.self)
            bytes.storeBytes(of: expectedAttachmentId.littleEndian, toByteOffset: 4, as: UInt64.self)
            bytes.storeBytes(of: resumeToken.littleEndian, toByteOffset: 12, as: UInt64.self)
        }
        sendMessage(type: .attach, payload: resumeAttachPayload)
        try waitUntil("attach acknowledgement after reconnect") {
            self.attachedTabId == tabId
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
        tabListReceived = false
        attachedTabId = nil
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
        connection?.send(
            content: header + payload,
            contentContext: .defaultStream,
            isComplete: false,
            completion: .contentProcessed { [weak self] error in
                if let error {
                    self?.setError("send failed: \(error)")
                }
            }
        )
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
        case .authOk:
            if let metadata = decodeValidationAuthOkMetadata(payload) {
                protocolVersion = metadata.protocolVersion
                transportCapabilities = metadata.transportCapabilities
                serverBuildId = metadata.serverBuildId
                serverInstanceId = metadata.serverInstanceId
                serverIdentityId = metadata.serverIdentityId
            }
            if let error = validateValidationAuthOkMetadata(payload) {
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
        case .tabList:
            tabListReceived = true
            tabs = WireCodec.decodeTabList(payload)
        case .tabCreated:
            guard payload.count >= 4 else { return }
            createdTabId = payload.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
            }
        case .attached:
            guard payload.count >= 4 else { return }
            attachedTabId = payload.withUnsafeBytes {
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
        case .detached, .tabExited:
            attachedTabId = nil
            attachmentId = nil
        case .uiRuntimeState, .uiAppearance:
            break
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
        let stateSummary = "authenticated=\(authenticated) heartbeatAckReceived=\(heartbeatAckReceived) tabListReceived=\(tabListReceived) tabs=\(tabs.count) createdTabId=\(String(describing: createdTabId)) attachedTabId=\(String(describing: attachedTabId)) attachmentId=\(String(describing: attachmentId)) resumeToken=\(String(describing: resumeToken)) buildId=\(serverBuildId ?? "nil") serverIdentityId=\(serverIdentityId ?? "nil") serverInstanceId=\(serverInstanceId ?? "nil") screenUpdateReceived=\(screenUpdateReceived) screen=\"\(screenSnippet)\" trace=\(messageTrace.joined(separator: ","))"
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

func resolveRemoteValidatorArgs() -> (host: String, port: UInt16, checkDiscovery: Bool) {
    var host = "127.0.0.1"
    var port: UInt16 = 7337
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
        case "--check-discovery":
            checkDiscovery = true
        default:
            break
        }
        index += 1
    }
    return (host, port, checkDiscovery)
}
