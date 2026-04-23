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

extension ClientWireMessageType {
    static var tabList: Self { .sessionList }
    static var tabCreated: Self { .sessionCreated }
    static var tabExited: Self { .sessionExited }
}

enum ClientWireErrorCode: UInt16, Equatable {
    case unknown = 0
    case authenticationFailed = 1
    case unknownSession = 2
    case failedCreateSession = 3
    case notAttached = 4
    case cannotDestroyLastSession = 5
    case attachmentAlreadyActive = 6
    case attachmentBelongsToDifferentSession = 7
    case attachmentResumeTokenMismatch = 8
    case attachmentResumeWindowExpired = 9
    case invalidResumeToken = 10
    case heartbeatTimeout = 11
}

extension ClientWireErrorCode {
    static var unknownTab: Self { .unknownSession }
    static var failedCreateTab: Self { .failedCreateSession }
    static var cannotDestroyLastTab: Self { .cannotDestroyLastSession }
    static var attachmentBelongsToDifferentTab: Self { .attachmentBelongsToDifferentSession }
}

enum ClientWireEffect: Equatable {
    case none
    case attach(UInt32)
}

enum ClientWireErrorKind: Equatable {
    case authenticationFailed
    case unknownSession
    case failedCreateSession
    case notAttached
    case attachmentResumeUnsupported
    case attachmentResumeWindowExpired
    case attachmentResumeTokenMismatch
    case invalidResumeToken
    case remote(String)

    var message: String {
        switch self {
        case .authenticationFailed:
            return "Authentication failed"
        case .unknownSession:
            return "unknown session"
        case .failedCreateSession:
            return "failed to create session"
        case .notAttached:
            return "not attached"
        case .attachmentResumeUnsupported:
            return "Remote server does not advertise attachment resume support"
        case .attachmentResumeWindowExpired:
            return "attachment resume window expired"
        case .attachmentResumeTokenMismatch:
            return "attachment resume token mismatch"
        case .invalidResumeToken:
            return "invalid resume token"
        case .remote(let message):
            return message
        }
    }

    var invalidatesResumeAttachment: Bool {
        switch self {
        case .attachmentResumeUnsupported,
                .attachmentResumeWindowExpired,
                .attachmentResumeTokenMismatch,
                .invalidResumeToken:
            return true
        case .authenticationFailed, .unknownSession, .failedCreateSession, .notAttached, .remote:
            return false
        }
    }

    static func uiTestNamed(_ raw: String) -> ClientWireErrorKind? {
        switch raw.lowercased() {
        case "authenticationfailed":
            return .authenticationFailed
        case "attachmentresumeunsupported":
            return .attachmentResumeUnsupported
        case "attachmentresumewindowexpired":
            return .attachmentResumeWindowExpired
        case "attachmentresumetokenmismatch":
            return .attachmentResumeTokenMismatch
        case "invalidresumetoken":
            return .invalidResumeToken
        default:
            return nil
        }
    }
}

extension ClientWireErrorKind {
    static var unknownTab: Self { .unknownSession }
    static var failedCreateTab: Self { .failedCreateSession }
}

struct AuthOkMetadata: Equatable {
    let protocolVersion: UInt16
    let transportCapabilities: UInt32
    let serverBuildId: String?
    let serverInstanceId: String?
    let serverIdentityId: String?
}

private let expectedRemoteProtocolVersion: UInt16 = 1
private let remoteCapabilityHeartbeat: UInt32 = 1 << 4
private let remoteCapabilityAttachmentResume: UInt32 = 1 << 5
private let remoteCapabilityDaemonIdentity: UInt32 = 1 << 6

struct ClientWireState: Equatable {
    var authenticated = false
    var protocolVersion: UInt16?
    var transportCapabilities: UInt32 = 0
    var serverBuildId: String?
    var serverInstanceId: String?
    var serverIdentityId: String?
    var tabs: [DecodedWireTabInfo] = []
    var screen: DecodedWireScreenState?
    var attachedTabId: UInt32?
    var attachmentId: UInt64?
    var resumeToken: UInt64?
    var lastErrorKind: ClientWireErrorKind?
    var lastError: String?

    init(
        authenticated: Bool = false,
        protocolVersion: UInt16? = nil,
        transportCapabilities: UInt32 = 0,
        serverBuildId: String? = nil,
        serverInstanceId: String? = nil,
        serverIdentityId: String? = nil,
        tabs: [DecodedWireTabInfo] = [],
        sessions: [DecodedWireSessionInfo]? = nil,
        screen: DecodedWireScreenState? = nil,
        attachedTabId: UInt32? = nil,
        attachedSessionId: UInt32? = nil,
        attachmentId: UInt64? = nil,
        resumeToken: UInt64? = nil,
        lastErrorKind: ClientWireErrorKind? = nil,
        lastError: String? = nil
    ) {
        self.authenticated = authenticated
        self.protocolVersion = protocolVersion
        self.transportCapabilities = transportCapabilities
        self.serverBuildId = serverBuildId
        self.serverInstanceId = serverInstanceId
        self.serverIdentityId = serverIdentityId
        self.tabs = sessions ?? tabs
        self.screen = screen
        self.attachedTabId = attachedSessionId ?? attachedTabId
        self.attachmentId = attachmentId
        self.resumeToken = resumeToken
        self.lastErrorKind = lastErrorKind
        self.lastError = lastError
    }

    var sessions: [DecodedWireSessionInfo] {
        get { tabs }
        set { tabs = newValue }
    }

    var attachedSessionId: UInt32? {
        get { attachedTabId }
        set { attachedTabId = newValue }
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
    case .unknownSession:
        return .unknownSession
    case .failedCreateSession:
        return .failedCreateSession
    case .notAttached:
        return .notAttached
    case .cannotDestroyLastSession:
        return .remote(message)
    case .attachmentAlreadyActive, .attachmentBelongsToDifferentSession:
        return .remote(message)
    case .attachmentResumeTokenMismatch:
        return .attachmentResumeTokenMismatch
    case .attachmentResumeWindowExpired:
        return .attachmentResumeWindowExpired
    case .invalidResumeToken:
        return .invalidResumeToken
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
        return AuthOkMetadata(
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
        return AuthOkMetadata(
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
    return AuthOkMetadata(
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
    if (metadata.transportCapabilities & remoteCapabilityAttachmentResume) == 0 {
        return "Remote server does not advertise attachment resume support"
    }
    if (metadata.transportCapabilities & remoteCapabilityDaemonIdentity) == 0 {
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

func serverIdentityMismatch(expectedIdentityId: String?, actualIdentityId: String?) -> Bool {
    guard let expectedIdentityId, !expectedIdentityId.isEmpty,
          let actualIdentityId, !actualIdentityId.isEmpty else {
        return false
    }
    return expectedIdentityId != actualIdentityId
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
                state.serverIdentityId = metadata.serverIdentityId
            }
            state.lastErrorKind = nil
            state.lastError = nil
            return .none
        case .authFail:
            state.lastErrorKind = .authenticationFailed
            state.lastError = ClientWireErrorKind.authenticationFailed.message
            return .none
        case .sessionList:
            state.tabs = WireCodec.decodeTabList(payload)
            return .none
        case .attached:
            guard payload.count >= 4 else { return .none }
            state.attachedTabId = payload.withUnsafeBytes {
                UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
            }
            state.attachmentId = payload.count >= 12 ? payload.withUnsafeBytes {
                UInt64(littleEndian: $0.loadUnaligned(fromByteOffset: 4, as: UInt64.self))
            } : nil
            state.resumeToken = payload.count >= 20 ? payload.withUnsafeBytes {
                UInt64(littleEndian: $0.loadUnaligned(fromByteOffset: 12, as: UInt64.self))
            } : nil
            return .none
        case .detached:
            state.attachedTabId = nil
            state.attachmentId = nil
            state.resumeToken = nil
            return .none
        case .sessionExited:
            if payload.count >= 4 {
                let exitedSessionId = payload.withUnsafeBytes {
                    UInt32(littleEndian: $0.loadUnaligned(fromByteOffset: 0, as: UInt32.self))
                }
                if state.attachedTabId == exitedSessionId {
                    state.attachedTabId = nil
                    state.attachmentId = nil
                    state.resumeToken = nil
                }
            } else {
                state.attachedTabId = nil
                state.attachmentId = nil
                state.resumeToken = nil
            }
            return .none
        case .sessionCreated:
            guard payload.count >= 4 else { return .none }
            return .none
        case .fullState:
            state.screen = WireCodec.decodeFullState(payload)
            return .none
        case .delta:
            guard var screen = state.screen else { return .none }
            guard WireCodec.applyDelta(payload, to: &screen) else { return .none }
            state.screen = screen
            return .none
        case .errorMsg:
            let kind = decodeClientWireError(payload)
            state.lastErrorKind = kind
            state.lastError = kind.message
            return .none
        }
    }
}
