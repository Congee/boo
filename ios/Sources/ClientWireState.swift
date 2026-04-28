import Foundation

enum ClientWireMessageType {
    case authOk
    case authFail
    case tabList
    case tabExited
    case fullState
    case delta
    case errorMsg
}

enum ClientWireErrorCode: UInt16, Equatable {
    case unknown = 0
    case authenticationFailed = 1
    case unknownTab = 2
    case failedCreateTab = 3
    case noActiveTab = 4
    case cannotDestroyLastTab = 5
    case heartbeatTimeout = 11
}

enum ClientWireErrorKind: Equatable {
    case authenticationFailed
    case unknownTab
    case failedCreateTab
    case noActiveTab
    case remote(String)

    var message: String {
        switch self {
        case .authenticationFailed:
            return "Authentication failed"
        case .unknownTab:
            return "unknown tab"
        case .failedCreateTab:
            return "failed to create tab"
        case .noActiveTab:
            return "no active tab"
        case .remote(let message):
            return message
        }
    }

    static func uiTestNamed(_ raw: String) -> ClientWireErrorKind? {
        switch raw.lowercased() {
        case "authenticationfailed":
            return .authenticationFailed
        default:
            return nil
        }
    }
}

struct AuthOkMetadata: Equatable {
    let protocolVersion: UInt16
    let transportCapabilities: UInt32
    let serverBuildId: String?
    let serverInstanceId: String?
}

private let expectedRemoteProtocolVersion: UInt16 = 1
private let remoteCapabilityHeartbeat: UInt32 = 1 << 4

struct ClientWireState: Equatable {
    var authenticated = false
    var protocolVersion: UInt16?
    var transportCapabilities: UInt32 = 0
    var serverBuildId: String?
    var serverInstanceId: String?
    var tabs: [DecodedWireTabInfo] = []
    var screen: DecodedWireScreenState?
    var lastErrorKind: ClientWireErrorKind?
    var lastError: String?

    init(
        authenticated: Bool = false,
        protocolVersion: UInt16? = nil,
        transportCapabilities: UInt32 = 0,
        serverBuildId: String? = nil,
        serverInstanceId: String? = nil,
        tabs: [DecodedWireTabInfo] = [],
        legacyTabs: [DecodedWireTabInfo]? = nil,
        screen: DecodedWireScreenState? = nil,
        lastErrorKind: ClientWireErrorKind? = nil,
        lastError: String? = nil
    ) {
        self.authenticated = authenticated
        self.protocolVersion = protocolVersion
        self.transportCapabilities = transportCapabilities
        self.serverBuildId = serverBuildId
        self.serverInstanceId = serverInstanceId
        self.tabs = legacyTabs ?? tabs
        self.screen = screen
        self.lastErrorKind = lastErrorKind
        self.lastError = lastError
    }
}

func decodeClientWireError(_ payload: Data) -> ClientWireErrorKind {
    guard payload.count >= 2 else { return .remote("Remote error") }
    let rawCode = payload.withUnsafeBytes {
        UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt16.self))
    }
    let message = String(data: payload.dropFirst(2), encoding: .utf8) ?? "Remote error"
    guard let code = ClientWireErrorCode(rawValue: rawCode) else {
        return .remote(message)
    }
    switch code {
    case .unknown:
        return .remote(message)
    case .authenticationFailed:
        return .authenticationFailed
    case .unknownTab:
        return .unknownTab
    case .failedCreateTab:
        return .failedCreateTab
    case .noActiveTab:
        return .noActiveTab
    case .cannotDestroyLastTab:
        return .remote(message)
    case .heartbeatTimeout:
        return .remote(message)
    }
}

func decodeAuthOkMetadata(_ payload: Data) -> AuthOkMetadata? {
    guard payload.count >= 6 else { return nil }
    let protocolVersion = payload.withUnsafeBytes {
        UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt16.self))
    }
    let transportCapabilities = payload.withUnsafeBytes {
        UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 2, as: UInt32.self))
    }
    guard payload.count >= 8 else {
        return AuthOkMetadata(
            protocolVersion: protocolVersion,
            transportCapabilities: transportCapabilities,
            serverBuildId: nil,
            serverInstanceId: nil
        )
    }
    let buildLength = payload.withUnsafeBytes {
        Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: 6, as: UInt16.self)))
    }
    guard payload.count >= 8 + buildLength else { return nil }
    let instanceLengthOffset = 8 + buildLength
    let serverBuildId = String(data: payload[8..<(8 + buildLength)], encoding: .utf8)
    guard payload.count >= instanceLengthOffset + 2 else {
        return AuthOkMetadata(
            protocolVersion: protocolVersion,
            transportCapabilities: transportCapabilities,
            serverBuildId: serverBuildId,
            serverInstanceId: nil
        )
    }
    let instanceLength = payload.withUnsafeBytes {
        Int(UInt16(littleEndian: $0.loadUnaligned(fromByteOffset: instanceLengthOffset, as: UInt16.self)))
    }
    guard payload.count >= instanceLengthOffset + 2 + instanceLength else { return nil }
    let serverInstanceId = String(
        data: payload[(instanceLengthOffset + 2)..<(instanceLengthOffset + 2 + instanceLength)],
        encoding: .utf8
    )
    return AuthOkMetadata(
        protocolVersion: protocolVersion,
        transportCapabilities: transportCapabilities,
        serverBuildId: serverBuildId,
        serverInstanceId: serverInstanceId
    )
}

func validateAuthOkMetadata(_ payload: Data) -> String? {
    guard let metadata = decodeAuthOkMetadata(payload) else {
        return "Remote handshake is malformed"
    }
    if metadata.protocolVersion != expectedRemoteProtocolVersion {
        return "Unsupported remote protocol version: \(metadata.protocolVersion)"
    }
    if (metadata.transportCapabilities & remoteCapabilityHeartbeat) == 0 {
        return "Remote server does not advertise heartbeat support"
    }
    if metadata.serverBuildId?.isEmpty != false {
        return "Remote handshake is missing server build metadata"
    }
    if metadata.serverInstanceId?.isEmpty != false {
        return "Remote handshake is missing server instance metadata"
    }
    return nil
}

enum ClientWireReducer {
    static func reduce(message: ClientWireMessageType, payload: Data, state: inout ClientWireState) {
        switch message {
        case .authOk:
            state.authenticated = true
            if let metadata = decodeAuthOkMetadata(payload) {
                state.protocolVersion = metadata.protocolVersion
                state.transportCapabilities = metadata.transportCapabilities
                state.serverBuildId = metadata.serverBuildId
                state.serverInstanceId = metadata.serverInstanceId
            }
            state.lastErrorKind = nil
            state.lastError = nil
        case .authFail:
            state.lastErrorKind = .authenticationFailed
            state.lastError = ClientWireErrorKind.authenticationFailed.message
        case .tabList:
            state.tabs = WireCodec.decodeTabList(payload)
        case .tabExited:
            break
        case .fullState:
            state.screen = WireCodec.decodeFullState(payload)
        case .delta:
            guard var screen = state.screen else { return }
            guard WireCodec.applyDelta(payload, to: &screen) else { return }
            state.screen = screen
        case .errorMsg:
            let kind = decodeClientWireError(payload)
            state.lastErrorKind = kind
            state.lastError = kind.message
        }
    }
}
