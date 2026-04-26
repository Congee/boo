import Foundation
import OSLog

enum BooTraceEvent: String {
    case remoteConnect = "remote.connect"
    case remoteRuntimeAction = "remote.runtime_action"
    case remoteFocusPane = "remote.focus_pane"
    case remoteSetViewedTab = "remote.set_viewed_tab"
    case remoteResizeSplit = "remote.resize_split"
    case remoteInput = "remote.input"
    case remotePaneUpdate = "remote.pane_update"
    case remoteRenderApply = "remote.render_apply"
    case remoteHeartbeatRtt = "remote.heartbeat_rtt"
    case remoteNoopRoundtrip = "remote.noop_roundtrip"
    case remoteActionAck = "remote.action_ack"
    case remoteOptimisticApply = "remote.optimistic_apply"
    case remoteReconcile = "remote.reconcile"
    case remoteRenderAck = "remote.render_ack"

    var signpostName: StaticString {
        switch self {
        case .remoteConnect: return "remote.connect"
        case .remoteRuntimeAction: return "remote.runtime_action"
        case .remoteFocusPane: return "remote.focus_pane"
        case .remoteSetViewedTab: return "remote.set_viewed_tab"
        case .remoteResizeSplit: return "remote.resize_split"
        case .remoteInput: return "remote.input"
        case .remotePaneUpdate: return "remote.pane_update"
        case .remoteRenderApply: return "remote.render_apply"
        case .remoteHeartbeatRtt: return "remote.heartbeat_rtt"
        case .remoteNoopRoundtrip: return "remote.noop_roundtrip"
        case .remoteActionAck: return "remote.action_ack"
        case .remoteOptimisticApply: return "remote.optimistic_apply"
        case .remoteReconcile: return "remote.reconcile"
        case .remoteRenderAck: return "remote.render_ack"
        }
    }
}

struct BooTraceFields {
    var interactionId: UInt64 = 0
    var viewId: UInt64 = 0
    var tabId: UInt32 = 0
    var paneId: UInt64 = 0
    var action: String = ""
    var route: String = "remote"
    var runtimeRevision: UInt64 = 0
    var viewRevision: UInt64 = 0
    var paneRevision: UInt64 = 0
    var elapsedMs: Double = 0

    var summary: String {
        "interaction_id=\(interactionId) view_id=\(viewId) tab_id=\(tabId) pane_id=\(paneId) action=\(action) route=\(route) runtime_revision=\(runtimeRevision) view_revision=\(viewRevision) pane_revision=\(paneRevision) elapsed_ms=\(String(format: "%.3f", elapsedMs))"
    }
}

struct BooTraceSpan {
    let event: BooTraceEvent
    let fields: BooTraceFields
    let startedAt: Date
    let state: OSSignpostIntervalState
}

struct BooTraceRecord {
    let event: BooTraceEvent
    let phase: String
    let sourceEvent: BooTraceEvent?
    let fields: BooTraceFields
}

struct BooRenderTraceTracker {
    private var interactionCounter: UInt64 = 0
    private var pendingInputTrace: BooTraceSpan?
    private var pendingFocusPaneTrace: BooTraceSpan?
    private var pendingRuntimeActionTrace: BooTraceSpan?

    mutating func nextInteractionId() -> UInt64 {
        interactionCounter &+= 1
        return interactionCounter
    }

    mutating func beginInput(_ fields: BooTraceFields) {
        pendingInputTrace = BooTrace.begin(.remoteInput, fields)
    }

    mutating func beginFocusPane(_ fields: BooTraceFields) {
        pendingFocusPaneTrace = BooTrace.begin(.remoteFocusPane, fields)
    }

    mutating func beginRuntimeAction(_ event: BooTraceEvent, _ fields: BooTraceFields) {
        pendingRuntimeActionTrace = BooTrace.begin(event, fields)
    }

    mutating func completeRenderApply(fields: BooTraceFields, tabId: UInt32?) {
        if let span = pendingFocusPaneTrace {
            pendingFocusPaneTrace = nil
            BooTrace.end(span, fields: fields)
        }
        if let span = pendingRuntimeActionTrace,
           span.fields.tabId == 0 || span.fields.tabId == (tabId ?? 0)
        {
            pendingRuntimeActionTrace = nil
            BooTrace.end(span, fields: fields)
        }
        if let span = pendingInputTrace {
            pendingInputTrace = nil
            BooTrace.end(span, fields: fields)
        }
    }
}

enum BooTrace {
    private static let logger = Logger(subsystem: "dev.boo.ios", category: "latency")
    private static let signposter = OSSignposter(logger: logger)
    private static var recorder: ((BooTraceRecord) -> Void)?

    static func installRecorder(_ record: ((BooTraceRecord) -> Void)?) {
        recorder = record
    }

    static func log(_ event: BooTraceEvent, _ fields: BooTraceFields) {
        logger.info("event=\(event.rawValue, privacy: .public) \(fields.summary, privacy: .public)")
        signposter.emitEvent(event.signpostName, "\(fields.summary, privacy: .public)")
        recorder?(BooTraceRecord(event: event, phase: "event", sourceEvent: nil, fields: fields))
    }

    static func debug(_ message: String) {
        logger.debug("\(message, privacy: .public)")
    }

    static func error(_ message: String) {
        logger.error("\(message, privacy: .public)")
    }

    static func begin(_ event: BooTraceEvent, _ fields: BooTraceFields) -> BooTraceSpan {
        logger.info("event=\(event.rawValue, privacy: .public) phase=begin \(fields.summary, privacy: .public)")
        let state = signposter.beginInterval(event.signpostName, "\(fields.summary, privacy: .public)")
        recorder?(BooTraceRecord(event: event, phase: "begin", sourceEvent: nil, fields: fields))
        return BooTraceSpan(event: event, fields: fields, startedAt: Date(), state: state)
    }

    static func end(_ span: BooTraceSpan, event: BooTraceEvent = .remoteRenderApply, fields endFields: BooTraceFields? = nil) {
        var fields = endFields ?? span.fields
        fields.interactionId = span.fields.interactionId
        fields.action = fields.action.isEmpty ? span.fields.action : fields.action
        fields.route = fields.route.isEmpty ? span.fields.route : fields.route
        fields.elapsedMs = Date().timeIntervalSince(span.startedAt) * 1000
        logger.info("event=\(event.rawValue, privacy: .public) phase=end source_event=\(span.event.rawValue, privacy: .public) \(fields.summary, privacy: .public)")
        signposter.endInterval(span.event.signpostName, span.state, "event=\(event.rawValue, privacy: .public) \(fields.summary, privacy: .public)")
        recorder?(BooTraceRecord(event: event, phase: "end", sourceEvent: span.event, fields: fields))
    }
}
