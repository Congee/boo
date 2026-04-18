import Foundation

enum ClientWireMessageType {
    case authOk
    case authFail
    case sessionList
    case attached
    case detached
    case sessionCreated
    case sessionExited
    case fullState
    case delta
    case errorMsg
}

enum ClientWireEffect: Equatable {
    case none
    case listSessions
    case attach(UInt32)
}

struct AuthOkMetadata: Equatable {
    let protocolVersion: UInt16
    let transportCapabilities: UInt32
    let serverBuildId: String?
    let serverInstanceId: String?
}

private let expectedRemoteProtocolVersion: UInt16 = 1
private let remoteCapabilityHmacAuth: UInt32 = 1 << 0
private let remoteCapabilityHeartbeat: UInt32 = 1 << 4
private let remoteCapabilityAttachmentResume: UInt32 = 1 << 5

struct ClientWireState: Equatable {
    var authenticated = false
    var protocolVersion: UInt16?
    var transportCapabilities: UInt32 = 0
    var serverBuildId: String?
    var serverInstanceId: String?
    var sessions: [DecodedWireSessionInfo] = []
    var screen: DecodedWireScreenState?
    var attachedSessionId: UInt32?
    var attachmentId: UInt64?
    var lastError: String?
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
    return AuthOkMetadata(
        protocolVersion: protocolVersion,
        transportCapabilities: transportCapabilities,
        serverBuildId: serverBuildId,
        serverInstanceId: String(
            data: payload[(instanceLengthOffset + 2)..<(instanceLengthOffset + 2 + instanceLength)],
            encoding: .utf8
        )
    )
}

func validateAuthOkMetadata(_ payload: Data, authRequired: Bool) -> String? {
    guard let metadata = decodeAuthOkMetadata(payload) else {
        return "Remote handshake is malformed"
    }
    if metadata.protocolVersion != expectedRemoteProtocolVersion {
        return "Unsupported remote protocol version: \(metadata.protocolVersion)"
    }
    if authRequired && (metadata.transportCapabilities & remoteCapabilityHmacAuth) == 0 {
        return "Remote server does not advertise HMAC authentication"
    }
    if (metadata.transportCapabilities & remoteCapabilityHeartbeat) == 0 {
        return "Remote server does not advertise heartbeat support"
    }
    if (metadata.transportCapabilities & remoteCapabilityAttachmentResume) == 0 {
        return "Remote server does not advertise attachment resume support"
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
    static func reduce(message: ClientWireMessageType, payload: Data, state: inout ClientWireState) -> ClientWireEffect {
        switch message {
        case .authOk:
            state.authenticated = true
            if let metadata = decodeAuthOkMetadata(payload) {
                state.protocolVersion = metadata.protocolVersion
                state.transportCapabilities = metadata.transportCapabilities
                state.serverBuildId = metadata.serverBuildId
                state.serverInstanceId = metadata.serverInstanceId
            }
            state.lastError = nil
            return .listSessions
        case .authFail:
            state.lastError = "Authentication failed"
            return .none
        case .sessionList:
            state.sessions = WireCodec.decodeSessionList(payload)
            return .none
        case .attached:
            guard payload.count >= 4 else { return .none }
            state.attachedSessionId = payload.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
            }
            state.attachmentId = payload.count >= 12 ? payload.withUnsafeBytes {
                UInt64(littleEndian: $0.loadUnaligned(fromByteOffset: 4, as: UInt64.self))
            } : nil
            return .none
        case .detached, .sessionExited:
            state.attachedSessionId = nil
            state.attachmentId = nil
            return .none
        case .sessionCreated:
            guard payload.count >= 4 else { return .none }
            let sessionId = payload.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
            }
            return .attach(sessionId)
        case .fullState:
            state.screen = WireCodec.decodeFullState(payload)
            return .none
        case .delta:
            guard var screen = state.screen else { return .none }
            guard WireCodec.applyDelta(payload, to: &screen) else { return .none }
            state.screen = screen
            return .none
        case .errorMsg:
            state.lastError = String(data: payload, encoding: .utf8) ?? "Remote error"
            return .none
        }
    }
}
